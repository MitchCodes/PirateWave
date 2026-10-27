//! Read-only endpoint diagnostics; never changes Windows volume or mute state.
#[derive(Debug)]
pub struct EndpointState {
    pub name: String,
    pub volume: f32,
    pub muted: bool,
}
#[cfg(windows)]
pub fn cable_endpoints() -> anyhow::Result<Vec<EndpointState>> {
    use windows::Win32::{
        Devices::Properties::DEVPKEY_Device_FriendlyName,
        Media::Audio::{Endpoints::IAudioEndpointVolume, *},
        System::Com::*,
    };
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let result = (|| -> anyhow::Result<Vec<EndpointState>> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let mut result = vec![];
            for flow in [eRender, eCapture] {
                let devices = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
                for i in 0..devices.GetCount()? {
                    let device = devices.Item(i)?;
                    let properties = device.OpenPropertyStore(STGM_READ)?;
                    let name = properties
                        .GetValue(&DEVPKEY_Device_FriendlyName as *const _ as *const _)?
                        .to_string();
                    if name.contains("CABLE") {
                        let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
                        result.push(EndpointState {
                            name,
                            volume: volume.GetMasterVolumeLevelScalar()?,
                            muted: volume.GetMute()?.as_bool(),
                        });
                    }
                }
            }
            Ok(result)
        })();
        CoUninitialize();
        result
    }
}
#[cfg(not(windows))]
pub fn cable_endpoints() -> anyhow::Result<Vec<EndpointState>> {
    Ok(vec![])
}
