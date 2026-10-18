use std::fs;
use std::path::Path;

use crate::config::Arch;
use crate::config::Cpu;
use crate::config::Firmware;
use crate::i18n::tt;

/// Что установщик узнаёт о машине, на которой запущен.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hardware {
    pub arch: Arch,
    pub cpu: Cpu,
    pub firmware: Firmware,
}

impl Default for Hardware {
    fn default() -> Self {
        Self {
            arch: Arch::X86_64,
            cpu: Cpu::Other,
            firmware: Firmware::Bios,
        }
    }
}

impl Hardware {
    pub fn detect() -> Self {
        let arch = arch(std::env::consts::ARCH);
        let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();

        Self {
            arch,
            cpu: cpu(&cpuinfo),
            firmware: firmware(arch, Path::new("/sys/firmware/efi").exists()),
        }
    }

    pub fn summary(self) -> String {
        let arch = match self.arch {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
        };
        let cpu = match self.cpu {
            Cpu::Intel => "Intel",
            Cpu::Amd => "AMD",
            Cpu::Other => tt!("CPU not detected", "процессор не определён"),
        };
        let firmware = match self.firmware {
            Firmware::Uefi => "UEFI",
            Firmware::Bios => "BIOS",
            Firmware::Hybrid => "BIOS+UEFI",
            Firmware::Uboot => "U-Boot",
        };

        format!("{arch} · {cpu} · {firmware}")
    }
}

pub fn arch(name: &str) -> Arch {
    match name {
        "aarch64" => Arch::Aarch64,
        _ => Arch::X86_64,
    }
}

/// Производитель по полю `vendor_id` из `/proc/cpuinfo` (на ARM его нет).
pub fn cpu(cpuinfo: &str) -> Cpu {
    let vendor = cpuinfo
        .lines()
        .find(|line| line.starts_with("vendor_id"))
        .and_then(|line| line.split(':').nth(1))
        .map(str::trim);

    match vendor {
        Some("GenuineIntel") => Cpu::Intel,
        Some("AuthenticAMD") => Cpu::Amd,
        _ => Cpu::Other,
    }
}

/// На x86 диск грузится и по BIOS, и по UEFI; на ARM — UEFI, если он есть, иначе U-Boot.
pub fn firmware(arch: Arch, efi: bool) -> Firmware {
    match (efi, arch) {
        (_, Arch::X86_64) => Firmware::Hybrid,
        (true, Arch::Aarch64) => Firmware::Uefi,
        (false, Arch::Aarch64) => Firmware::Uboot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_architecture_comes_from_the_target_name() {
        assert_eq!(arch("aarch64"), Arch::Aarch64);
        assert_eq!(arch("x86_64"), Arch::X86_64);
    }

    #[test]
    fn the_vendor_is_read_from_cpuinfo() {
        let intel = "processor : 0\nvendor_id : GenuineIntel\nmodel name : x\n";
        let amd = "processor : 0\nvendor_id\t: AuthenticAMD\n";

        assert_eq!(cpu(intel), Cpu::Intel);
        assert_eq!(cpu(amd), Cpu::Amd);
        assert_eq!(cpu("processor : 0\nCPU part : 0xd0c\n"), Cpu::Other);
        assert_eq!(cpu(""), Cpu::Other);
    }

    #[test]
    fn the_firmware_follows_efi_and_the_architecture() {
        assert_eq!(firmware(Arch::X86_64, true), Firmware::Hybrid);
        assert_eq!(firmware(Arch::Aarch64, true), Firmware::Uefi);
        assert_eq!(firmware(Arch::X86_64, false), Firmware::Hybrid);
        assert_eq!(firmware(Arch::Aarch64, false), Firmware::Uboot);
    }

    #[test]
    fn the_summary_names_the_architecture_the_vendor_and_the_firmware() {
        let hardware = Hardware {
            arch: Arch::Aarch64,
            cpu: Cpu::Other,
            firmware: Firmware::Uboot,
        };

        assert_eq!(
            hardware.summary(),
            "aarch64 · процессор не определён · U-Boot"
        );
    }
}
