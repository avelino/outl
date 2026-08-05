//! Fingerprint of the *physical* machine the device store sits on.
//!
//! The device store is outside every workspace, but it is not outside
//! Migration Assistant, a Time Machine restore, a VM or container image,
//! `chezmoi` / Mackup, or `$HOME` on NFS. A cloned store carries the same
//! `machine-id`, so without an independent signal both machines answer
//! "yes, that claim is mine" and nothing self-heals.
//!
//! The signal is an OS-provided identifier of the hardware, hashed before
//! it is written so the raw value never lands on disk. Where the platform
//! exposes none the answer is `None`, and every caller treats that as
//! *inconclusive* rather than as "cloned" — a false fork on each open
//! would be worse than the hole it closes.

use std::sync::OnceLock;

use super::hash_hex;

/// Hashed host identifier, or `None` where this platform exposes none
/// (iOS, unknown targets). Computed at most once per process.
pub(super) fn host_fingerprint() -> Option<&'static str> {
    static FP: OnceLock<Option<String>> = OnceLock::new();
    FP.get_or_init(|| {
        platform_host_id()
            .map(|raw| raw.trim().to_string())
            .filter(|raw| !raw.is_empty())
            .map(|raw| hash_hex(raw.as_bytes()))
    })
    .as_deref()
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn platform_host_id() -> Option<String> {
    // systemd regenerates `/etc/machine-id` on first boot of a cloned
    // image, which is exactly the property we want.
    ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
}

#[cfg(target_os = "macos")]
fn platform_host_id() -> Option<String> {
    // `IOPlatformUUID` is burned into the hardware, so a Migration
    // Assistant transfer or a restored Time Machine backup lands on a
    // different value — the clone we need to catch.
    let out = std::process::Command::new("/usr/sbin/ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains("IOPlatformUUID"))?;
    // `      "IOPlatformUUID" = "0000-…"` → the 4th `"`-delimited field.
    line.split('"').nth(3).map(str::to_string)
}

#[cfg(target_os = "windows")]
fn platform_host_id() -> Option<String> {
    let out = std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains("MachineGuid"))?;
    line.split_whitespace().last().map(str::to_string)
}

/// iOS above all: no subprocess, no `/etc/machine-id`, and a restored
/// device backup is precisely the hazard. Documented as a known gap in
/// the parent module rather than papered over with a weak signal.
#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "windows"
)))]
fn platform_host_id() -> Option<String> {
    None
}
