//! Explicit, DLL-specific research oracle. Reads DSP state without modifying it.
//! Build/run only as x86 against the documented target binary. Not shipped.
#![allow(deprecated)]
#[cfg(not(all(windows, target_pointer_width = "32")))]
fn main() {
    eprintln!("Build with --target i686-pc-windows-msvc to run this research oracle.");
}
#[cfg(all(windows, target_pointer_width = "32"))]
fn main() -> anyhow::Result<()> {
    use std::{ffi::c_void, io::Write};
    use vst::api::{AEffect, HostCallbackProc};
    extern "C" fn host(
        _: *mut AEffect,
        opcode: i32,
        _: i32,
        _: isize,
        _: *mut c_void,
        _: f32,
    ) -> isize {
        match opcode {
            1 => 2400,
            16 => 48000,
            17 => 128,
            32 => 1,
            _ => 0,
        }
    }
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() >= 3,
        "voice_trace <DLL> <output directory> [input hz] [transpose st] [harmonic|transition|glide|vowel|noise]"
    );
    let directory = std::path::Path::new(&args[2]);
    std::fs::create_dir_all(directory)?;
    let hz = args
        .get(3)
        .map(|s| s.parse::<f32>())
        .transpose()?
        .unwrap_or(430.0);
    let transpose = args
        .get(4)
        .map(|s| s.parse::<f32>())
        .transpose()?
        .unwrap_or(0.0);
    unsafe {
        let library = libloading::Library::new(&args[1])?;
        let entry: libloading::Symbol<extern "C" fn(HostCallbackProc) -> *mut AEffect> =
            library.get(b"VSTPluginMain")?;
        let base = *entry as usize - 0x22070;
        let effect = entry(host);
        anyhow::ensure!(!effect.is_null(), "No AEffect");
        let e = &*effect;
        anyhow::ensure!(
            e.numParams == 75 && e.numInputs == 2 && e.numOutputs == 2,
            "Unexpected target profile"
        );
        let object = e.object as *const u8;
        anyhow::ensure!(
            !object.is_null() && *(object as *const usize) == base + 0x39634,
            "Target object vtable mismatch"
        );
        let dispatch = |opcode, opt, value| {
            (e.dispatcher)(effect, opcode, 0, value, std::ptr::null_mut(), opt)
        };
        dispatch(0, 0.0, 0);
        dispatch(10, 48000.0, 0);
        dispatch(11, 0.0, 128);
        for (id, value) in [
            (1, 2.0 / 28.0),
            (2, 0.0),
            (4, 1.0),
            (61, 0.0),
            (74, (transpose + 12.0) / 24.0),
        ] {
            (e.setParameter)(effect, id, value);
        }
        dispatch(12, 0.0, 1);
        dispatch(71, 0.0, 0);
        let dsp = *((object.add(0x29c)) as *const *const u8);
        anyhow::ensure!(
            !dsp.is_null() && *(dsp as *const usize) == base + 0x3cffc,
            "DSP vtable mismatch"
        );
        let read = |offset| std::ptr::read_unaligned(dsp.add(offset) as *const f64);
        let int = |offset| std::ptr::read_unaligned(dsp.add(offset) as *const i32);
        let mut summary = serde_json::json!({"rate_630":read(0x630),"a_int":int(0x828),"a_frac":read(0x820),"b_int":int(0x838),"b_frac":read(0x830),"fade":read(0x840),"write":int(0x650),"mode_488":*dsp.add(0x488),"flag_48b":*dsp.add(0x48b),"initial_delay":e.initialDelay});
        let mut csv = std::io::BufWriter::new(std::fs::File::create(directory.join("trace.csv"))?);
        writeln!(csv,"sample,input,output,period_640,period_648,ratio_7c8,ratio_7c0,write,a_int,a_frac,b_int,b_frac,fade,mode_488,prepared_l,prepared_r,delayed_period,delayed_ratio,current_ratio,delayed_last_period")?;
        let mut raw = std::io::BufWriter::new(std::fs::File::create(directory.join("audio.f32"))?);
        let mut input = [0.0f32; 1];
        let mut right = [0.0f32; 1];
        let mut left_out = [0.0f32; 1];
        let mut right_out = [0.0f32; 1];
        let inputs = [input.as_ptr(), right.as_ptr()];
        let mut outputs = [left_out.as_mut_ptr(), right_out.as_mut_ptr()];
        let kind = args.get(5).map(String::as_str).unwrap_or("harmonic");
        let mut continuous_phase = 0.0_f64;
        let mut noise_seed = 12345_u32;
        for sample in 0..48000 {
            let phase = std::f32::consts::TAU * hz * sample as f32 / 48000.0;
            input[0] = 0.2 * phase.sin() + 0.08 * (2.0 * phase).sin() + 0.04 * (3.0 * phase).sin();
            match kind {
                "transition" => {
                    let frequency = if sample < 24000 { hz as f64 } else { 250.0 };
                    continuous_phase += std::f64::consts::TAU * frequency / 48000.0;
                    input[0] = (0.2 * continuous_phase.sin()
                        + 0.08 * (2.0 * continuous_phase).sin()
                        + 0.04 * (3.0 * continuous_phase).sin())
                        as f32;
                }
                "glide" => {
                    continuous_phase += std::f64::consts::TAU
                        * (hz as f64 + 70.0 * sample as f64 / 48000.0)
                        / 48000.0;
                    input[0] = (0.2 * continuous_phase.sin()
                        + 0.08 * (2.0 * continuous_phase).sin()
                        + 0.04 * (3.0 * continuous_phase).sin())
                        as f32;
                }
                "vowel" => {
                    input[0] = (1..=16)
                        .map(|h| {
                            let frequency = hz * h as f32;
                            let envelope = 0.4
                                + 3.0 * (-((frequency - 700.0) / 200.0).powi(2)).exp()
                                + 2.0 * (-((frequency - 1200.0) / 250.0).powi(2)).exp();
                            0.1 * envelope / (h as f32) * (phase * h as f32).sin()
                        })
                        .sum();
                }
                "noise" => {
                    noise_seed = noise_seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    input[0] = noise_seed as i32 as f32 / i32::MAX as f32 * 0.1;
                }
                _ => {}
            }
            right[0] = input[0];
            (e.processReplacing)(effect, inputs.as_ptr(), outputs.as_mut_ptr(), 1);
            raw.write_all(&left_out[0].to_le_bytes())?;
            writeln!(
                csv,
                "{sample},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                input[0],
                left_out[0],
                read(0x640),
                read(0x648),
                read(0x7c8),
                read(0x7c0),
                int(0x650),
                int(0x828),
                read(0x820),
                int(0x838),
                read(0x830),
                read(0x840),
                *dsp.add(0x488),
                *(*(dsp.add(0x11b8) as *const *const f64))
                    .add(int(0x650).rem_euclid(5696) as usize),
                *(*(dsp.add(0x11bc) as *const *const f64))
                    .add(int(0x650).rem_euclid(5696) as usize),
                *(*(dsp.add(0x11c0) as *const *const f64))
                    .add((int(0x650) - 1280).rem_euclid(5696) as usize),
                *(*(dsp.add(0x11c8) as *const *const f64))
                    .add((int(0x650) - 1280).rem_euclid(5696) as usize),
                *(*(dsp.add(0x11c8) as *const *const f64))
                    .add(int(0x650).rem_euclid(5696) as usize),
                *(*(dsp.add(0x11c4) as *const *const f64))
                    .add((int(0x650) - 1280).rem_euclid(5696) as usize)
            )?;
        }
        summary["final_mode_488"] = serde_json::json!(*dsp.add(0x488));
        summary["final_period"] = serde_json::json!(read(0x640));
        summary["final_ratio"] = serde_json::json!(read(0x7c0));
        std::fs::write(
            directory.join("summary.json"),
            serde_json::to_vec_pretty(&summary)?,
        )?;
        dispatch(72, 0.0, 0);
        dispatch(12, 0.0, 0);
        dispatch(1, 0.0, 0);
    }
    Ok(())
}
