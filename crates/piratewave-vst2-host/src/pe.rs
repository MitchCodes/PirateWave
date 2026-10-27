//! Small, bounded PE header reader; never loads or executes the target.
use anyhow::{Context, Result};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Architecture {
    X86,
    X64,
    Arm64,
    Other(u16),
}
pub fn architecture(path: &Path) -> Result<Architecture> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("Opening plugin {}", path.display()))?;
    let length = file.metadata()?.len();
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos).context("Truncated DOS header")?;
    anyhow::ensure!(&dos[..2] == b"MZ", "Plugin is not a Windows PE DLL");
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    anyhow::ensure!(
        offset >= 64 && offset.saturating_add(26) <= length,
        "Invalid PE header offset"
    );
    file.seek(SeekFrom::Start(offset))?;
    let mut pe = [0u8; 26];
    file.read_exact(&mut pe)?;
    anyhow::ensure!(&pe[..4] == b"PE\0\0", "Invalid PE signature");
    let machine = u16::from_le_bytes(pe[4..6].try_into().unwrap());
    let magic = u16::from_le_bytes(pe[24..26].try_into().unwrap());
    anyhow::ensure!(
        magic == 0x10b || magic == 0x20b,
        "Invalid PE optional header"
    );
    Ok(match machine {
        0x14c if magic == 0x10b => Architecture::X86,
        0x8664 if magic == 0x20b => Architecture::X64,
        0xaa64 => Architecture::Arm64,
        _ => Architecture::Other(machine),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifies_architecture_and_rejects_invalid_headers() {
        let path = std::env::temp_dir().join(format!("piratewave-pe-{}.dll", std::process::id()));
        for (machine, magic, expected) in [
            (0x14cu16, 0x10bu16, Architecture::X86),
            (0x8664, 0x20b, Architecture::X64),
        ] {
            let mut bytes = vec![0u8; 128];
            bytes[..2].copy_from_slice(b"MZ");
            bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
            bytes[64..68].copy_from_slice(b"PE\0\0");
            bytes[68..70].copy_from_slice(&machine.to_le_bytes());
            bytes[88..90].copy_from_slice(&magic.to_le_bytes());
            std::fs::write(&path, &bytes).unwrap();
            assert_eq!(architecture(&path).unwrap(), expected);
        }
        std::fs::write(&path, b"MZ").unwrap();
        assert!(architecture(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
