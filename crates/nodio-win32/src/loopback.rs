use parking_lot::Mutex;
use std::future::Future;
use std::mem::ManuallyDrop;
use std::pin::Pin;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{mpsc, Arc};
use std::task::{Context, Poll, Waker};

use crate::render::RenderClient;
use nodio_core::Uuid;
use pollster::FutureExt as _;
use windows::core::{implement, IUnknown, Interface, Result, GUID, HRESULT};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Media::Audio::{
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IMMDevice, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::BLOB;
use windows::Win32::System::Ole::VT_BLOB;
use windows::Win32::System::Threading::CreateEventW;
use windows::Win32::{
    Media::Audio::{IAudioCaptureClient, IAudioClient, AUDCLNT_SHAREMODE_SHARED},
    Media::MediaFoundation::*,
    System::Com::StructuredStorage::PROPVARIANT,
};

pub struct LoopbackCapture {
    target_pid: u32,
    include_process_tree: bool,

    format: WAVEFORMATEXTENSIBLE,

    sample_ready_key: u64,
    audio_client: Option<IAudioClient>,
    capture_client: Option<IAudioCaptureClient>,
    ev_sample_ready: HANDLE,
    sample_ready_result: Option<IMFAsyncResult>,

    queue_id: u32,
}

impl LoopbackCapture {
    fn new(target_pid: u32, format: WAVEFORMATEXTENSIBLE) -> Self {
        Self {
            format,
            target_pid,
            include_process_tree: true,
            sample_ready_key: 0,
            audio_client: None,
            capture_client: None,
            ev_sample_ready: HANDLE(0),
            sample_ready_result: None,
            queue_id: 0,
        }
    }

    pub unsafe fn get_next_packet_size(&self) -> Result<u32> {
        self.capture_client.as_ref().unwrap().GetNextPacketSize()
    }

    pub unsafe fn get_buffer(&mut self) -> Result<BufferPacket> {
        let mut data_ptr = null_mut::<u8>();

        let mut frames: u32 = 0;
        let mut dw_capture_flags: u32 = 0;
        let mut device_position: u64 = 0;
        let mut qpc_position: u64 = 0;

        self.capture_client.as_ref().unwrap().GetBuffer(
            &mut data_ptr as *mut *mut u8,
            &mut frames as *mut u32,
            &mut dw_capture_flags as *mut u32,
            &mut device_position as *mut u64,
            &mut qpc_position as *mut u64,
        )?;

        let num_block_align: u16 =
            self.format.Format.nChannels * self.format.Format.wBitsPerSample / 8u16;

        Ok(BufferPacket {
            data: data_ptr,
            frames,
            size: frames * num_block_align as u32,
        })
    }

    pub unsafe fn release_buffer(&mut self, frames: u32) -> Result<()> {
        self.capture_client
            .as_ref()
            .unwrap()
            .ReleaseBuffer(frames)?;

        self.sample_ready_key =
            MFPutWaitingWorkItem(self.ev_sample_ready, 0, &self.sample_ready_result)?;

        Ok(())
    }

    pub unsafe fn start(&mut self, callback: Box<dyn Fn(&mut LoopbackCapture)>) {
        let mut task_id: u32 = 0;

        MFStartup(MF_SDK_VERSION << 16 | MF_API_VERSION, MFSTARTUP_LITE).unwrap();
        MFLockSharedWorkQueue("Capture", 0, &mut task_id, &mut self.queue_id).unwrap();

        let mut audio_params = AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: self.target_pid,
                    ProcessLoopbackMode: if self.include_process_tree {
                        PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE
                    } else {
                        PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE
                    },
                },
            },
        };

        let activate_params = ManuallyDrop::new(PROPVARIANT_0_0 {
            vt: VT_BLOB.0 as u16,
            Anonymous: PROPVARIANT_0_0_0 {
                blob: BLOB {
                    cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                    pBlobData: (&mut audio_params) as *mut AUDIOCLIENT_ACTIVATION_PARAMS as *mut u8,
                },
            },
            ..Default::default()
        });

        let activate_params: PROPVARIANT = PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: activate_params,
            },
        };

        let completion_handler = CompletionHandler::new();
        let completion_handler_interface: IActivateAudioInterfaceCompletionHandler =
            completion_handler.clone().into();

        let op = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID as *const GUID,
            &activate_params,
            &completion_handler_interface,
        )
        .unwrap();

        completion_handler.block_on();

        let mut activate_result = HRESULT(0);
        let mut activated_interface: Option<IUnknown> = None;

        op.GetActivateResult(
            &mut activate_result as *mut HRESULT,
            &mut activated_interface as *mut Option<IUnknown>,
        )
        .unwrap();

        activate_result.ok().unwrap();

        let activated_interface = activated_interface.unwrap();
        let audio_client: IAudioClient = core::mem::transmute(activated_interface);
        self.audio_client = Some(audio_client);
        let audio_client = self.audio_client.as_ref().unwrap();

        audio_client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK
                    | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                0,
                0,
                &self.format as *const WAVEFORMATEXTENSIBLE as _, //capture_format,
                null(),
            )
            .unwrap();

        let capture_client = audio_client.GetService::<IAudioCaptureClient>().unwrap();
        self.capture_client = Some(capture_client);

        let sample_capturer: IMFAsyncCallback = AsyncCallback::create(
            self.queue_id,
            Some(callback),
            self as *const LoopbackCapture as *mut LoopbackCapture,
        );

        let ev_sample_ready = CreateEventW(null(), false, false, None).unwrap();

        let async_result = MFCreateAsyncResult(None, &sample_capturer, None).unwrap();
        self.sample_ready_result = Some(async_result);

        audio_client.SetEventHandle(ev_sample_ready).unwrap();

        let (start_capture, receiver) =
            AsyncCallback::with_receiver(MFASYNC_CALLBACK_QUEUE_MULTITHREADED);

        MFPutWorkItem2(
            MFASYNC_CALLBACK_QUEUE_MULTITHREADED,
            0,
            &start_capture,
            None,
        )
        .unwrap();

        receiver.recv().ok();

        audio_client.Start().unwrap();

        self.sample_ready_key =
            MFPutWaitingWorkItem(ev_sample_ready, 0, &self.sample_ready_result).unwrap();

        self.ev_sample_ready = ev_sample_ready;
    }

    pub unsafe fn stop(&mut self) {
        if self.sample_ready_key != 0 {
            MFCancelWorkItem(self.sample_ready_key).unwrap();
            self.sample_ready_key = 0;
        }

        if let Some(client) = &self.audio_client {
            client.Stop().unwrap();
            self.audio_client = None;
        }

        self.sample_ready_result = None;

        if self.queue_id != 0 {
            MFUnlockWorkQueue(self.queue_id).unwrap();
            self.queue_id = 0;
        }
    }
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
#[derive(Clone)]
struct CompletionHandler {
    completed: Arc<AtomicBool>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl CompletionHandler {
    fn new() -> CompletionHandler {
        CompletionHandler {
            completed: Arc::new(AtomicBool::new(false)),
            waker: Arc::new(Mutex::new(None)),
        }
    }
}

impl IActivateAudioInterfaceCompletionHandler_Impl for CompletionHandler {
    fn ActivateCompleted(&self, _: &Option<IActivateAudioInterfaceAsyncOperation>) -> Result<()> {
        let self_ptr = self as *const CompletionHandler as *mut CompletionHandler;
        unsafe {
            (*self_ptr).completed.store(true, Ordering::SeqCst);
        }

        if let Some(waker) = self.waker.lock().take() {
            waker.wake()
        };

        Ok(())
    }
}

impl Future for CompletionHandler {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.completed.load(Ordering::SeqCst) {
            Poll::Ready(())
        } else {
            self.waker.lock().replace(cx.waker().clone());
            Poll::Pending
        }
    }
}

#[implement(IMFAsyncCallback)]
struct AsyncCallback {
    queue_id: u32,
    sender: Option<mpsc::Sender<()>>,
    callback: Option<Box<dyn Fn(&mut LoopbackCapture)>>,
    capture_ptr: *mut LoopbackCapture,
}

impl AsyncCallback {
    fn create(
        queue_id: u32,
        callback: Option<Box<dyn Fn(&mut LoopbackCapture)>>,
        capture_ptr: *mut LoopbackCapture,
    ) -> IMFAsyncCallback {
        AsyncCallback {
            queue_id,
            sender: None,
            callback,
            capture_ptr,
        }
        .into()
    }

    fn with_receiver(queue_id: u32) -> (IMFAsyncCallback, Receiver<()>) {
        let (tx, rx) = mpsc::channel();
        (
            AsyncCallback {
                queue_id,
                sender: Some(tx),
                callback: None,
                capture_ptr: null_mut(),
            }
            .into(),
            rx,
        )
    }
}

impl IMFAsyncCallback_Impl for AsyncCallback {
    fn GetParameters(&self, pdwflags: *mut u32, pdwqueue: *mut u32) -> Result<()> {
        unsafe {
            *pdwflags = 0;
            *pdwqueue = self.queue_id;
        }
        Ok(())
    }

    fn Invoke(&self, _result: &Option<IMFAsyncResult>) -> Result<()> {
        if let Some(sender) = self.sender.as_ref() {
            sender.send(()).expect("send() failed.");
        }

        if let Some(c) = self.callback.as_ref() {
            c(unsafe { &mut *self.capture_ptr });
        }
        Ok(())
    }
}

#[repr(C)]
pub struct BufferPacket {
    pub data: *const u8,
    pub frames: u32,
    pub size: u32,
}

pub struct LoopbackSession {
    pub src_id: Uuid,
    pub dst_id: Uuid,
    capture: Box<LoopbackCapture>,
}

impl Drop for LoopbackSession {
    fn drop(&mut self) {
        unsafe {
            self.capture.stop();
        }
    }
}

impl LoopbackSession {
    pub fn start(
        src_id: Uuid,
        dst_id: Uuid,
        process_id: u32,
        target_device: &IMMDevice,
    ) -> Result<Self> {
        let render_client = RenderClient::new(target_device)?;
        let mut capture = Box::new(LoopbackCapture::new(
            process_id,
            *render_client.wave_format(),
        ));

        let frame_callback = Box::new(move |capture: &mut LoopbackCapture| unsafe {
            let frames = capture
                .get_next_packet_size()
                .expect("Failed to get next packet size");

            if frames == 0 {
                return;
            }

            let packet = capture.get_buffer().expect("Failed to get buffer");

            render_client.render_frames(packet.data, packet.frames).ok();

            capture
                .release_buffer(frames)
                .expect("Failed to release buffer");
        });

        unsafe {
            capture.start(frame_callback);
        }

        Ok(Self {
            src_id,
            dst_id,
            capture,
        })
    }
}
