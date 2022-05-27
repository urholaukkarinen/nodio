#![deny(clippy::all)]
mod com;
mod context;
mod custom;
mod device;
mod enumerator;
mod loopback;
mod node;
mod render;
mod session;

use widestring::U16CStr;
use windows::core::PWSTR;

pub use context::Win32Context;

fn pwstr_to_string(pwstr: PWSTR) -> String {
    if pwstr.is_null() {
        String::default()
    } else {
        unsafe { U16CStr::from_ptr_str(pwstr.0).to_string_lossy() }
    }
}

type Callback<T> = Box<dyn Fn(T) + Send + Sync>;
