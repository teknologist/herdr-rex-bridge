//! Convert Rex snapshots to VT bytes accepted by unmodified Herdr.
use anyhow::{Result, ensure};
use ghostty_sys::*;
use std::{ffi::c_void, mem::size_of, ptr};
const SNAPSHOT_TAGS: std::ops::RangeInclusive<u16> = 1..=7;
const SNAPSHOT_FINISH: u16 = 6;
/// What a snapshot may take to import (Rex's own bound).
const MAX_SNAPSHOT_BYTES: usize = 64 << 20;
/// What a snapshot's screen may hold.
const MAX_SNAPSHOT_CELLS: usize = 65_536;

/// The snapshot with records of unknown tags left out, as newer Ghostty
/// decoders skip them: each is checked (CRC32C) first, so a damaged one still
/// fails. Known records stay byte for byte, for Ghostty to validate.
fn known_snapshot_records(bytes: &[u8]) -> anyhow::Result<std::borrow::Cow<'_, [u8]>> {
    use anyhow::{Context, ensure};
    ensure!(
        bytes.len() <= MAX_SNAPSHOT_BYTES,
        "the snapshot exceeds {MAX_SNAPSHOT_BYTES} bytes"
    );
    ensure!(
        bytes.len() >= 10 && &bytes[..8] == b"GHOSTSNP",
        "not a Ghostty snapshot"
    );
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    ensure!(
        version == 1,
        "unsupported Ghostty snapshot version {version}"
    );
    let mut known: Option<Vec<u8>> = None;
    let mut at = 10;
    loop {
        ensure!(bytes.len() - at >= 10, "truncated snapshot record");
        let header = &bytes[at..at + 10];
        let tag = u16::from_le_bytes([header[0], header[1]]);
        let len = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as usize;
        let end = (at + 10)
            .checked_add(len)
            .filter(|end| *end <= bytes.len())
            .context("truncated snapshot record")?;
        if SNAPSHOT_TAGS.contains(&tag) {
            if let Some(known) = &mut known {
                known.extend_from_slice(&bytes[at..end]);
            }
        } else {
            let crc = u32::from_le_bytes([header[6], header[7], header[8], header[9]]);
            ensure!(
                crc32c(&[&header[..6], &bytes[at + 10..end]]) == crc,
                "snapshot record {tag} fails its checksum"
            );
            known.get_or_insert_with(|| bytes[..at].to_vec());
        }
        at = end;
        if tag == SNAPSHOT_FINISH {
            break;
        }
    }
    ensure!(at == bytes.len(), "bytes follow the snapshot's FINISH");
    Ok(known.map_or(std::borrow::Cow::Borrowed(bytes), std::borrow::Cow::Owned))
}

/// CRC32C (Castagnoli), as Ghostty's snapshot records use.
fn crc32c(parts: &[&[u8]]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0; 256];
        let mut i = 0;
        while i < 256 {
            let mut v = i as u32;
            let mut bit = 0;
            while bit < 8 {
                v = if v & 1 == 0 {
                    v >> 1
                } else {
                    (v >> 1) ^ 0x82f6_3b78
                };
                bit += 1;
            }
            table[i] = v;
            i += 1;
        }
        table
    };
    let mut crc = u32::MAX;
    for b in parts.iter().flat_map(|p| p.iter()) {
        crc = (crc >> 8) ^ TABLE[((crc ^ u32::from(*b)) & 0xff) as usize];
    }
    !crc
}

/// Restores a terminal from a whole snapshot with Ghostty's decoder, which
/// validates every record (checksums, order, counts, contents).
fn decode_snapshot(bytes: &[u8]) -> anyhow::Result<GhosttyTerminal> {
    unsafe {
        let mut decoder: GhosttySnapshotDecoder = std::ptr::null_mut();
        let r = ghostty_snapshot_decoder_new_buf(
            std::ptr::null(),
            &mut decoder,
            bytes.as_ptr(),
            bytes.len(),
        );
        anyhow::ensure!(
            r == GHOSTTY_SUCCESS && !decoder.is_null(),
            "cannot start decoding the snapshot ({r})"
        );
        let mut term: GhosttyTerminal = std::ptr::null_mut();
        let r = ghostty_snapshot_decoder_decode(decoder, &mut term);
        let mut used = 0usize;
        let offset = ghostty_snapshot_decoder_get(
            decoder,
            GHOSTTY_SNAPSHOT_DECODER_DATA_SOURCE_OFFSET,
            &mut used as *mut _ as *mut c_void,
        );
        ghostty_snapshot_decoder_free(decoder);
        if r != GHOSTTY_SUCCESS || term.is_null() {
            anyhow::bail!("invalid snapshot ({r})");
        }
        if offset != GHOSTTY_SUCCESS || used != bytes.len() {
            ghostty_terminal_free(term);
            anyhow::bail!(
                "the snapshot decoder stopped at byte {used} of {}",
                bytes.len()
            );
        }
        Ok(term)
    }
}

struct Terminal(GhosttyTerminal);
impl Drop for Terminal {
    fn drop(&mut self) {
        unsafe { ghostty_terminal_free(self.0) };
    }
}
fn check(r: GhosttyResult, what: &str) -> Result<()> {
    ensure!(r == GHOSTTY_SUCCESS, "{what}: libghostty error {r}");
    Ok(())
}
fn allocated(call: impl FnOnce(*mut *mut u8, *mut usize) -> GhosttyResult) -> Result<Vec<u8>> {
    let (mut p, mut n) = (ptr::null_mut(), 0);
    check(call(&mut p, &mut n), "format terminal")?;
    let bytes = if n == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(p, n).to_vec() }
    };
    unsafe { ghostty_free(ptr::null(), p, n) };
    Ok(bytes)
}
fn format(term: GhosttyTerminal, emit: GhosttyFormatterFormat, extras: bool) -> Result<Vec<u8>> {
    let screen = GhosttyFormatterScreenExtra {
        size: size_of::<GhosttyFormatterScreenExtra>(),
        cursor: extras,
        style: extras,
        hyperlink: extras,
        protection: extras,
        kitty_keyboard: extras,
        charsets: extras,
    };
    let extra = GhosttyFormatterTerminalExtra {
        size: size_of::<GhosttyFormatterTerminalExtra>(),
        palette: extras,
        modes: extras,
        scrolling_region: extras,
        tabstops: extras,
        pwd: extras,
        keyboard: extras,
        screen,
    };
    let opts = GhosttyFormatterTerminalOptions {
        size: size_of::<GhosttyFormatterTerminalOptions>(),
        emit,
        unwrap: false,
        trim: true,
        extra,
        selection: ptr::null(),
    };
    let mut f = ptr::null_mut();
    check(
        unsafe { ghostty_formatter_terminal_new(ptr::null(), &mut f, term, opts) },
        "create formatter",
    )?;
    let out = allocated(|p, n| unsafe { ghostty_formatter_format_alloc(f, ptr::null(), p, n) });
    unsafe { ghostty_formatter_free(f) };
    out
}
fn without_screen_switches(bytes: &[u8]) -> Vec<u8> {
    // The formatter's independent 47/1047/1049 bits can disagree with the
    // actual screen. Choose it explicitly above; synchronized output belongs
    // to the old display and must not freeze a new one.
    let skip: &[&[u8]] = &[b"\x1b[?47h", b"\x1b[?1047h", b"\x1b[?1049h", b"\x1b[?2026h"];
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if let Some(seq) = skip.iter().find(|s| bytes[at..].starts_with(s)) {
            at += seq.len();
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    out
}
pub fn snapshot_vt(bytes: &[u8]) -> Result<Vec<u8>> {
    let records = known_snapshot_records(bytes)?;
    let mut decoder = ptr::null_mut();
    check(
        unsafe {
            ghostty_snapshot_decoder_new_buf(
                ptr::null(),
                &mut decoder,
                records.as_ptr(),
                records.len(),
            )
        },
        "create snapshot decoder",
    )?;
    let retain = true;
    check(
        unsafe {
            ghostty_snapshot_decoder_set(
                decoder,
                GHOSTTY_SNAPSHOT_DECODER_OPT_RETAIN_CONTINUATION,
                &retain as *const _ as *const c_void,
            )
        },
        "retain continuation",
    )?;
    let mut raw = ptr::null_mut();
    let result = unsafe { ghostty_snapshot_decoder_decode(decoder, &mut raw) };
    unsafe { ghostty_snapshot_decoder_free(decoder) };
    check(result, "decode snapshot")?;
    ensure!(!raw.is_null(), "snapshot returned no terminal");
    let term = Terminal(raw);
    let off = false;
    unsafe {
        ghostty_terminal_set(
            raw,
            GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_MEDIUM_FILE,
            &off as *const _ as *const c_void,
        );
        ghostty_terminal_set(
            raw,
            GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_MEDIUM_TEMP_FILE,
            ptr::null(),
        );
        ghostty_terminal_set(
            raw,
            GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_MEDIUM_SHARED_MEM,
            &off as *const _ as *const c_void,
        );
    }
    let mut out = b"\x1bc".to_vec(); // Fresh repaint, including keyboard and mouse defaults.
    let mut active = GHOSTTY_TERMINAL_SCREEN_PRIMARY;
    check(
        unsafe {
            ghostty_terminal_get(
                raw,
                GHOSTTY_TERMINAL_DATA_ACTIVE_SCREEN,
                &mut active as *mut _ as *mut c_void,
            )
        },
        "active screen",
    )?;
    if active == GHOSTTY_TERMINAL_SCREEN_ALTERNATE {
        // Populate the primary screen too: leaving an editor after reconnect
        // must reveal the shell and its history, rather than an empty screen.
        let primary = Terminal(decode_snapshot(&records)?);
        let switch = b"\x18\x1b[?1049l\x1b[?1047l\x1b[?47l";
        unsafe { ghostty_terminal_vt_write(primary.0, switch.as_ptr(), switch.len()) };
        out.extend(without_screen_switches(&format(
            primary.0,
            GHOSTTY_FORMATTER_FORMAT_VT,
            true,
        )?));
        out.extend(b"\x1b[?1049h");
    }
    out.extend(without_screen_switches(&format(
        raw,
        GHOSTTY_FORMATTER_FORMAT_VT,
        true,
    )?));
    let mut title = GhosttyString::default();
    if unsafe {
        ghostty_terminal_get(
            raw,
            GHOSTTY_TERMINAL_DATA_TITLE,
            &mut title as *mut _ as *mut c_void,
        )
    } == GHOSTTY_SUCCESS
        && !title.ptr.is_null()
    {
        out.extend(b"\x1b]2;");
        out.extend(
            unsafe { std::slice::from_raw_parts(title.ptr, title.len) }
                .iter()
                .copied()
                .filter(|b| *b >= 0x20 && *b != 0x7f),
        );
        out.extend(b"\x1b\\");
    }
    out.extend(allocated(|p, n| unsafe {
        ghostty_terminal_continuation_alloc(raw, ptr::null(), p, n)
    })?);
    drop(term);
    Ok(out)
}

/// Rex already answered device queries. Never ask Herdr to answer again.
/// Kitty APCs are omitted: stock Herdr cannot import their snapshot records,
/// and forwarding file/shared-memory media could access the viewer's machine.
#[derive(Default)]
pub struct OutputFilter {
    held: Vec<u8>,
    discarding: bool,
}
impl OutputFilter {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len());
        for &b in bytes {
            if self.held.is_empty() {
                if b == 0x1b {
                    self.held.push(b);
                } else {
                    out.push(b);
                }
                continue;
            }
            let last = self.held.last().copied();
            if self.held.len() < 65536 {
                self.held.push(b);
            } else {
                self.discarding = true;
            }
            let kind = self.held.get(1).copied();
            let done = match kind {
                None => false,
                Some(b'[') => self.held.len() > 2 && (0x40..=0x7e).contains(&b),
                Some(b']' | b'P' | b'_') => {
                    (kind == Some(b']') && b == 7)
                        || (last == Some(0x1b) && b == b'\\')
                        || b == 0x18
                        || b == 0x1a
                }
                _ => true,
            };
            // Remember the terminator even when a large string is being discarded.
            if self.discarding && !done {
                let n = self.held.len();
                self.held[n - 1] = b;
            }
            if done {
                if !self.discarding && !query(&self.held) {
                    display_sequence(&self.held, &mut out);
                }
                self.held.clear();
                self.discarding = false;
            }
        }
        out
    }
}
fn display_sequence(seq: &[u8], out: &mut Vec<u8>) {
    // Rex already sends in-band resize reports. Stock Herdr would generate
    // a second report if its display copy enabled mode 2048.
    if let Some(body) = seq.strip_prefix(b"\x1b[?")
        && let Some((&last, params)) = body.split_last()
        && matches!(last, b'h' | b'l')
    {
        let modes: Vec<&[u8]> = params.split(|b| *b == b';').collect();
        if modes.contains(&b"2048".as_slice()) {
            let modes: Vec<_> = modes.into_iter().filter(|m| *m != b"2048").collect();
            if !modes.is_empty() {
                out.extend(b"\x1b[?");
                for (i, m) in modes.iter().enumerate() {
                    if i > 0 {
                        out.push(b';');
                    }
                    out.extend_from_slice(m);
                }
                out.push(last);
            }
            return;
        }
    }
    out.extend_from_slice(seq);
}

fn query(s: &[u8]) -> bool {
    if let Some(p) = s.strip_prefix(b"\x1b[") {
        let (&last, args) = p.split_last().unwrap_or((&0, &[]));
        return matches!(last, b'c' | b'n')
            || (last == b'p' && args.ends_with(b"$"))
            || (last == b'q' && args.starts_with(b">"))
            || (last == b'u' && args == b"?")
            || (last == b't' && [b"14".as_slice(), b"16", b"18", b"19"].contains(&args));
    }
    if let Some(body) = s.strip_prefix(b"\x1b]") {
        let command = body.split(|b| *b == b';').next().unwrap_or_default();
        return body.contains(&b'?')
            && matches!(
                command,
                b"4" | b"10"
                    | b"11"
                    | b"12"
                    | b"13"
                    | b"14"
                    | b"15"
                    | b"16"
                    | b"17"
                    | b"18"
                    | b"19"
                    | b"52"
            );
    }
    s.starts_with(b"\x1bP$q") || s.starts_with(b"\x1bP+q") || s.starts_with(b"\x1b_G")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn new() -> Terminal {
        let mut t = ptr::null_mut();
        check(
            unsafe { ghostty_terminal_new(ptr::null(), &mut t, 30, 5) },
            "new",
        )
        .unwrap();
        let n: usize = 65536;
        unsafe {
            ghostty_terminal_set(
                t,
                GHOSTTY_TERMINAL_OPT_CONTINUATION_MAX_BYTES,
                &n as *const _ as *const c_void,
            );
        }
        Terminal(t)
    }
    fn write(t: &Terminal, b: &[u8]) {
        unsafe { ghostty_terminal_vt_write(t.0, b.as_ptr(), b.len()) };
    }
    fn snapshot(t: &Terminal) -> Vec<u8> {
        allocated(|p, n| unsafe { ghostty_snapshot_encode_alloc(t.0, ptr::null(), p, n) }).unwrap()
    }
    fn text(t: &Terminal) -> String {
        String::from_utf8(format(t.0, GHOSTTY_FORMATTER_FORMAT_PLAIN, false).unwrap()).unwrap()
    }
    fn pos(t: &Terminal) -> (u16, u16) {
        let (mut x, mut y) = (0, 0);
        unsafe {
            ghostty_terminal_get(
                t.0,
                GHOSTTY_TERMINAL_DATA_CURSOR_X,
                &mut x as *mut _ as *mut c_void,
            );
            ghostty_terminal_get(
                t.0,
                GHOSTTY_TERMINAL_DATA_CURSOR_Y,
                &mut y as *mut _ as *mut c_void,
            );
        }
        (x, y)
    }
    #[test]
    fn snapshot_replays_screen_history_cursor_and_continuation() {
        let src = new();
        for i in 0..14 {
            write(&src, format!("history-{i}\r\n").as_bytes());
        }
        write(
            &src,
            b"\x1b[31mred\x1b[0m\r\n\xe4\xb8\xad wide\x1b[3;4H\x1b[1;3",
        );
        let vt = snapshot_vt(&snapshot(&src)).unwrap();
        let dst = new();
        write(&dst, &vt);
        assert_eq!(text(&src), text(&dst));
        assert_eq!(pos(&src), pos(&dst));
        write(&src, b"2mX");
        write(&dst, b"2mX");
        assert_eq!(text(&src), text(&dst));
        assert_eq!(
            format(src.0, GHOSTTY_FORMATTER_FORMAT_VT, true).unwrap(),
            format(dst.0, GHOSTTY_FORMATTER_FORMAT_VT, true).unwrap()
        );
    }
    #[test]
    fn snapshot_replays_alternate_screen_and_input_modes() {
        let src = new();
        write(
            &src,
            b"primary\x1b[?1049halt screen\x1b[?2004h\x1b[?1006h\x1b[?1003h\x1b[4;7H",
        );
        let dst = new();
        write(&dst, &snapshot_vt(&snapshot(&src)).unwrap());
        assert_eq!(text(&src), text(&dst));
        assert_eq!(pos(&src), pos(&dst));
        for mode in [2004, 1006, 1003] {
            let mut a = GhosttyTerminalModeConfig {
                mode: (mode | 0x8000),
                value: false,
            };
            let mut b = a;
            unsafe {
                ghostty_terminal_get(
                    src.0,
                    GHOSTTY_TERMINAL_DATA_MODE,
                    &mut a as *mut _ as *mut c_void,
                );
                ghostty_terminal_get(
                    dst.0,
                    GHOSTTY_TERMINAL_DATA_MODE,
                    &mut b as *mut _ as *mut c_void,
                );
            }
            assert_eq!(a.value, b.value);
        }
        write(&src, b"\x1b[?1049l");
        write(&dst, b"\x1b[?1049l");
        assert_eq!(
            text(&src),
            text(&dst),
            "leaving the alternate screen restores the primary shell"
        );
        assert_eq!(pos(&src), pos(&dst));
    }
    #[test]
    fn filter_removes_queries_and_kitty_at_every_chunk_boundary() {
        let input=b"before\x1b[6n\x1b[c\x1b[>q\x1b[?2004$p\x1b[16t\x1b[?u\x1b]11;?\x07\x1bP$qm\x1b\\\x1b_Ga=T,t=f;L2V0Yy9wYXNzd2Q=\x1b\\after\x1b[31mred\x1b[0m\x1b]2;why?\x07";
        let want = b"beforeafter\x1b[31mred\x1b[0m\x1b]2;why?\x07";
        for n in 1..input.len() {
            let mut f = OutputFilter::default();
            let out: Vec<u8> = input.chunks(n).flat_map(|c| f.feed(c)).collect();
            assert_eq!(out, want, "chunk {n}");
        }
    }
    #[test]
    fn filter_bounds_large_strings_and_recovers_after_st() {
        let mut f = OutputFilter::default();
        assert!(f.feed(b"\x1b_G").is_empty());
        for _ in 0..70 {
            assert!(f.feed(&vec![b'a'; 1024]).is_empty());
        }
        assert_eq!(f.feed(b"\x1b\\kept"), b"kept");
    }
    #[test]
    fn display_never_generates_a_second_in_band_resize_report() {
        let mut filter = OutputFilter::default();
        assert_eq!(
            filter.feed(b"\x1b[?2048h\x1b[?2004;2048;1003h\x1b[?2048l"),
            b"\x1b[?2004;1003h"
        );
    }

    #[test]
    fn malformed_snapshot_is_rejected() {
        for b in [b"".as_slice(), b"GHOSTSNP\x01\x00", b"not a snapshot"] {
            assert!(snapshot_vt(b).is_err());
        }
    }
}
