use std::marker::PhantomData;

use std::ptr::null;
use windows::core::Result;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

thread_local!(static COM_INITIALIZED: ComInitialized = {
    unsafe {
        let result = CoInitializeEx(null(), COINIT_APARTMENTTHREADED);


        match result {
            Err(err) if err.code() != RPC_E_CHANGED_MODE => {
                panic!("Failed to initialize COM: {}", err.message())

            },
            _ => ComInitialized {
                result,
                _ptr: PhantomData,
            }
        }
    }
});

struct ComInitialized {
    result: Result<()>,
    _ptr: PhantomData<*mut ()>,
}

impl Drop for ComInitialized {
    #[inline]
    fn drop(&mut self) {
        if self.result.is_ok() {
            unsafe { CoUninitialize() }
        }
    }
}

#[inline]
pub fn ensure_com_initialized() {
    COM_INITIALIZED.with(|_| {});
}
