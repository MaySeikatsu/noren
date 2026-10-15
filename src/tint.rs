//! Preview tint: the preview pane's background a shade off the terminal's,
//! so picker and preview read as two areas. The terminal background is
//! asked for with OSC 11 (`ESC ] 11 ; ? ST`); terminals (and zellij)
//! answer `rgb:RRRR/GGGG/BBBB`. No answer within 100 ms → no tint.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::process::{Command, Stdio};

/// Run `stty <args>` on the terminal.
fn stty(tty: &File, args: &[&str]) -> Option<String> {
    let out = Command::new("stty")
        .args(args)
        .stdin(tty.try_clone().ok()?)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The terminal's background colour, if it answers OSC 11.
pub fn terminal_bg() -> Option<(u8, u8, u8)> {
    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    let saved = stty(&tty, &["-g"])?;
    // Raw, no echo, reads return after ≤100 ms without input.
    stty(&tty, &["raw", "-echo", "min", "0", "time", "1"])?;
    let mut reply = Vec::new();
    if tty.write_all(b"\x1b]11;?\x1b\\").is_ok() {
        let mut buf = [0u8; 64];
        // A few reads: the answer may arrive in pieces; stop at BEL / ST.
        for _ in 0..4 {
            match tty.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => reply.extend_from_slice(&buf[..n]),
            }
            if reply.ends_with(b"\x07") || reply.ends_with(b"\x1b\\") {
                break;
            }
        }
    }
    let _ = stty(&tty, &[&saved]);
    parse_reply(&String::from_utf8_lossy(&reply))
}

/// `…rgb:RRRR/GGGG/BBBB…` (1–4 hex digits per channel) → 8-bit RGB.
fn parse_reply(reply: &str) -> Option<(u8, u8, u8)> {
    let rgb = reply.split("rgb:").nth(1)?;
    let mut channels = rgb.split(['/', '\x07', '\x1b']).take(3).map(|h| {
        let h = h.trim();
        let v = u32::from_str_radix(h, 16).ok()?;
        let max = (1u32 << (4 * h.len().clamp(1, 4))) - 1;
        Some((v * 255 / max) as u8)
    });
    Some((channels.next()??, channels.next()??, channels.next()??))
}

/// A shade off `bg`: darker, or lighter when it is (nearly) black already.
pub fn shade((r, g, b): (u8, u8, u8)) -> String {
    let luma = (r as u32 * 2126 + g as u32 * 7152 + b as u32 * 722) / 10_000;
    let f = |c: u8| -> u8 {
        if luma < 24 {
            c.saturating_add(18)
        } else if luma < 128 {
            (c as u32 * 3 / 4) as u8
        } else {
            (c as u32 * 92 / 100) as u8
        }
    };
    format!("#{:02x}{:02x}{:02x}", f(r), f(g), f(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_osc11_replies() {
        assert_eq!(
            parse_reply("\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\"),
            Some((30, 30, 46))
        );
        assert_eq!(parse_reply("\x1b]11;rgb:ff/80/00\x07"), Some((255, 128, 0)));
        assert_eq!(parse_reply("garbage"), None);
        assert_eq!(parse_reply(""), None);
    }

    #[test]
    fn shades_darker_or_lighter_on_black() {
        assert_eq!(shade((30, 30, 46)), "#161622"); // dark theme: darker
        assert_eq!(shade((250, 250, 250)), "#e6e6e6"); // light theme: darker
        assert_eq!(shade((0, 0, 0)), "#121212"); // black: lighter
    }
}
