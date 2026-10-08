//! Just enough of windows-sys for localtime.rs to compile on Linux; every call fails.
#![allow(non_snake_case, non_camel_case_types, clippy::missing_safety_doc)]
pub mod Win32 {
    pub mod Foundation {
        #[derive(Default, Clone, Copy)]
        pub struct FILETIME { pub dwLowDateTime: u32, pub dwHighDateTime: u32 }
        #[derive(Default, Clone, Copy)]
        pub struct SYSTEMTIME { pub wYear: u16, pub wMonth: u16, pub wDayOfWeek: u16, pub wDay: u16, pub wHour: u16, pub wMinute: u16, pub wSecond: u16, pub wMilliseconds: u16 }
    }
    pub mod System {
        pub mod Console {
            pub const ENABLE_ECHO_INPUT: u32 = 4;
            pub const STD_INPUT_HANDLE: u32 = u32::MAX - 9;
            pub unsafe fn GetStdHandle(_: u32) -> *mut core::ffi::c_void { core::ptr::null_mut() }
            pub unsafe fn GetConsoleMode(_: *mut core::ffi::c_void, _: *mut u32) -> i32 { 0 }
            pub unsafe fn SetConsoleMode(_: *mut core::ffi::c_void, _: u32) -> i32 { 0 }
        }
        pub mod Time {
            use super::super::Foundation::*;
            #[derive(Clone, Copy)]
            pub struct DYNAMIC_TIME_ZONE_INFORMATION { pub Bias: i32, pub StandardName: [u16; 32], pub StandardDate: SYSTEMTIME, pub StandardBias: i32, pub DaylightName: [u16; 32], pub DaylightDate: SYSTEMTIME, pub DaylightBias: i32, pub TimeZoneKeyName: [u16; 128], pub DynamicDaylightTimeDisabled: bool }
            impl Default for DYNAMIC_TIME_ZONE_INFORMATION { fn default() -> Self { unsafe { std::mem::zeroed() } } }
            pub unsafe fn EnumDynamicTimeZoneInformation(_: u32, _: *mut DYNAMIC_TIME_ZONE_INFORMATION) -> u32 { 1 }
            pub unsafe fn GetDynamicTimeZoneInformation(_: *mut DYNAMIC_TIME_ZONE_INFORMATION) -> u32 { u32::MAX }
            pub unsafe fn FileTimeToSystemTime(_: *const FILETIME, _: *mut SYSTEMTIME) -> i32 { 0 }
            pub unsafe fn SystemTimeToFileTime(_: *const SYSTEMTIME, _: *mut FILETIME) -> i32 { 0 }
            pub unsafe fn SystemTimeToTzSpecificLocalTimeEx(_: *const DYNAMIC_TIME_ZONE_INFORMATION, _: *const SYSTEMTIME, _: *mut SYSTEMTIME) -> i32 { 0 }
            pub unsafe fn TzSpecificLocalTimeToSystemTimeEx(_: *const DYNAMIC_TIME_ZONE_INFORMATION, _: *const SYSTEMTIME, _: *mut SYSTEMTIME) -> i32 { 0 }
        }
    }
}
