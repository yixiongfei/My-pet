//! 系统正在出多大的声：默认输出设备的峰值电平（WASAPI `IAudioMeterInformation`），0–1。
//!
//! 用来给「在不在放歌」做第二道确认（Spotify 标题说在播、但一点声都没有——那是
//! Launcher 或者静音，不算），以及听出歌到高潮。裸 COM vtable 调用，不引 windows crate：
//! 只用到四个方法。每次采样开一次关一次，几毫秒；不保留任何指针跨线程。

#[cfg(target_os = "windows")]
pub fn peak() -> Option<f32> {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Guid {
        d1: u32,
        d2: u16,
        d3: u16,
        d4: [u8; 8],
    }
    const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Guid {
        Guid { d1, d2, d3, d4 }
    }
    // mmdeviceapi.h / endpointvolume.h（Windows SDK 10.0.26100）
    const CLSID_MM_DEVICE_ENUMERATOR: Guid = guid(0xBCDE0395, 0xE52F, 0x467C, [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E]);
    const IID_IMM_DEVICE_ENUMERATOR: Guid = guid(0xA95664D2, 0x9614, 0x4F35, [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6]);
    const IID_IAUDIO_METER_INFORMATION: Guid = guid(0xC02216F6, 0x8C67, 0x4B5B, [0x9D, 0x00, 0xD0, 0x08, 0xE7, 0x3E, 0x00, 0x64]);
    const CLSCTX_ALL: u32 = 0x17;
    const COINIT_MULTITHREADED: u32 = 0x0;
    const E_RENDER: u32 = 0;
    const E_CONSOLE: u32 = 0;

    type Unknown = *mut c_void;
    // vtable 顺序照头文件：IUnknown 三个之后是各自的方法
    #[repr(C)]
    struct EnumeratorVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(Unknown) -> u32,
        enum_audio_endpoints: usize,
        get_default_audio_endpoint: unsafe extern "system" fn(Unknown, u32, u32, *mut Unknown) -> i32,
    }
    #[repr(C)]
    struct DeviceVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(Unknown) -> u32,
        activate: unsafe extern "system" fn(Unknown, *const Guid, u32, *mut c_void, *mut Unknown) -> i32,
    }
    #[repr(C)]
    struct MeterVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(Unknown) -> u32,
        get_peak_value: unsafe extern "system" fn(Unknown, *mut f32) -> i32,
    }
    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> i32;
        fn CoUninitialize();
        fn CoCreateInstance(clsid: *const Guid, outer: *mut c_void, ctx: u32, iid: *const Guid, out: *mut Unknown) -> i32;
    }

    // SAFETY: 全部按 Win32 / COM 文档：每个接口在用完后 Release；CoInitialize 成对；
    // 指针只在本函数内使用
    unsafe {
        let init = CoInitializeEx(std::ptr::null_mut(), COINIT_MULTITHREADED);
        // S_OK / S_FALSE 都算成功；RPC_E_CHANGED_MODE（线程已是 STA）也能继续用
        let inited = init >= 0;
        let mut enumerator: Unknown = std::ptr::null_mut();
        let mut result = None;
        if CoCreateInstance(&CLSID_MM_DEVICE_ENUMERATOR, std::ptr::null_mut(), CLSCTX_ALL, &IID_IMM_DEVICE_ENUMERATOR, &mut enumerator) >= 0 && !enumerator.is_null() {
            let evt = &**(enumerator as *mut *mut EnumeratorVtbl);
            let mut device: Unknown = std::ptr::null_mut();
            if (evt.get_default_audio_endpoint)(enumerator, E_RENDER, E_CONSOLE, &mut device) >= 0 && !device.is_null() {
                let dvt = &**(device as *mut *mut DeviceVtbl);
                let mut meter: Unknown = std::ptr::null_mut();
                if (dvt.activate)(device, &IID_IAUDIO_METER_INFORMATION, CLSCTX_ALL, std::ptr::null_mut(), &mut meter) >= 0 && !meter.is_null() {
                    let mvt = &**(meter as *mut *mut MeterVtbl);
                    let mut v = 0.0f32;
                    if (mvt.get_peak_value)(meter, &mut v) >= 0 {
                        result = Some(v.clamp(0.0, 1.0));
                    }
                    (mvt.release)(meter);
                }
                (dvt.release)(device);
            }
            (evt.release)(enumerator);
        }
        if inited {
            CoUninitialize();
        }
        result
    }
}

#[cfg(not(target_os = "windows"))]
pub fn peak() -> Option<f32> {
    None
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "读真机的声卡电平，看一眼数值"]
    fn 读一次电平() {
        println!("peak = {:?}", super::peak());
    }
}
