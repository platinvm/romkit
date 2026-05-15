//! romkit for browsers: plain wasm32 imports and exports over one open [`Rom`], with JSON
//! results and no binding generator.
//!
//! The host (typically a Web Worker) provides the ROM through `romkit.read_at`, e.g. with
//! `FileReaderSync` over `Blob.slice`, so only the bytes romkit asks for are ever read. Every
//! call ends with exactly one `romkit.emit_result` carrying `{"ok": value}` or
//! `{"error": "message"}`; calls that produce files first pass each to `romkit.emit_file`.
//!
//! | Export                        | Result                                   |
//! |-------------------------------|------------------------------------------|
//! | `romkit_open(size)`           | [`Overview`](romkit::Overview)           |
//! | `romkit_files()`              | `[FileInfo]`                             |
//! | `romkit_archive(path)`        | `[FileInfo]`, the archive's members      |
//! | `romkit_read(path, member)`   | file emitted; its size (`member` < 0: whole file) |
//! | `romkit_assets()`             | `[path]`                                 |
//! | `romkit_extract(path)`        | asset emitted; its size                  |
//! | `romkit_export()`             | every asset emitted; the manifest        |
//!
//! Strings are passed as UTF-8 in memory from `romkit_alloc` (pointer, length). Sizes are `f64`
//! so that JavaScript numbers pass as they are. Host functions must not call back into romkit.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;

use romkit::{ByteSource, Error, Result, Rom};
use serde::Serialize;

#[link(wasm_import_module = "romkit")]
unsafe extern "C" {
    /// Copy `len` bytes of the ROM at `offset` to `dst`; return 0 on success.
    fn read_at(offset: f64, dst: *mut u8, len: usize) -> i32;
    /// Receive one output file; return 0 to go on, anything else to abort.
    fn emit_file(path: *const u8, path_len: usize, data: *const u8, data_len: usize) -> i32;
    /// Receive the JSON result of the current call.
    fn emit_result(json: *const u8, len: usize);
}

/// The ROM, as the host reads it.
struct HostRom {
    size: u64,
}

impl ByteSource for HostRom {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        if offset
            .checked_add(buf.len() as u64)
            .is_none_or(|end| end > self.size)
        {
            return Err(Error::Host("read past the end of the ROM".into()));
        }
        // SAFETY: `buf` is writable linear memory for `buf.len()` bytes.
        match unsafe { read_at(offset as f64, buf.as_mut_ptr(), buf.len()) } {
            0 => Ok(()),
            code => Err(Error::Host(format!("read_at failed with code {code}"))),
        }
    }
}

fn emit(path: &str, data: &[u8]) -> Result<()> {
    // SAFETY: both regions are live for the duration of the call.
    match unsafe { emit_file(path.as_ptr(), path.len(), data.as_ptr(), data.len()) } {
        0 => Ok(()),
        _ => Err(Error::Host("aborted by the host".into())),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum Response<T> {
    Ok(T),
    Error(String),
}

/// Emit `result` as the call's response. Returns 0 on success and 1 on error.
fn respond<T: Serialize>(result: Result<T>) -> i32 {
    let (response, status) = match result {
        Ok(value) => (Response::Ok(value), 0),
        Err(error) => (Response::Error(error.to_string()), 1),
    };
    let json = serde_json::to_vec(&response).unwrap_or_else(|error| {
        format!(r#"{{"error":"result does not serialize: {error}"}}"#).into_bytes()
    });
    // SAFETY: `json` is live for the duration of the call.
    unsafe { emit_result(json.as_ptr(), json.len()) };
    status
}

thread_local! {
    static ROM: RefCell<Option<Rom<'static>>> = const { RefCell::new(None) };
}

/// Run `call` on the open ROM and respond with its result.
fn with_rom<T: Serialize>(call: impl FnOnce(&Rom<'static>) -> Result<T>) -> i32 {
    ROM.with_borrow(|rom| {
        respond(
            rom.as_ref()
                .ok_or_else(|| Error::Host("no ROM is open".into()))
                .and_then(call),
        )
    })
}

/// # Safety
/// `ptr..ptr + len` must be memory from [`romkit_alloc`].
unsafe fn arg<'a>(ptr: *const u8, len: usize) -> Result<&'a str> {
    if len == 0 {
        return Ok("");
    }
    // SAFETY: guaranteed by the caller.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(bytes).map_err(|_| Error::Host("argument is not UTF-8".into()))
}

/// Reserve `len` bytes for the host to write an argument into.
#[unsafe(no_mangle)]
pub extern "C" fn romkit_alloc(len: usize) -> *mut u8 {
    let mut buffer = std::mem::ManuallyDrop::new(Vec::<u8>::with_capacity(len));
    buffer.as_mut_ptr()
}

/// # Safety
/// `ptr` and `len` must come from one [`romkit_alloc`] call, and be freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn romkit_free(ptr: *mut u8, len: usize) {
    // SAFETY: guaranteed by the caller.
    drop(unsafe { Vec::from_raw_parts(ptr, 0, len) });
}

/// Open a ROM of `size` bytes in place of any open one.
#[unsafe(no_mangle)]
pub extern "C" fn romkit_open(size: f64) -> i32 {
    ROM.set(None);
    respond(Rom::open(HostRom { size: size as u64 }).and_then(|rom| {
        let overview = rom.overview()?;
        ROM.set(Some(rom));
        Ok(overview)
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn romkit_close() {
    ROM.set(None);
}

#[unsafe(no_mangle)]
pub extern "C" fn romkit_files() -> i32 {
    with_rom(Rom::files)
}

/// # Safety
/// `path` must satisfy [`arg`]'s contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn romkit_archive(path: *const u8, len: usize) -> i32 {
    // SAFETY: forwarded from the caller.
    let path = unsafe { arg(path, len) };
    with_rom(|rom| rom.archive(path?))
}

/// # Safety
/// `path` must satisfy [`arg`]'s contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn romkit_read(path: *const u8, len: usize, member: i32) -> i32 {
    // SAFETY: forwarded from the caller.
    let path = unsafe { arg(path, len) };
    with_rom(|rom| {
        let path = path?;
        let data = match usize::try_from(member) {
            Ok(index) => rom.read_member(path, index)?,
            Err(_) => rom.read_file(path)?,
        };
        emit(path, &data)?;
        Ok(data.len())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn romkit_assets() -> i32 {
    with_rom(|rom| game(rom)?.assets())
}

/// # Safety
/// `path` must satisfy [`arg`]'s contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn romkit_extract(path: *const u8, len: usize) -> i32 {
    // SAFETY: forwarded from the caller.
    let path = unsafe { arg(path, len) };
    with_rom(|rom| {
        let path = path?;
        let data = game(rom)?.extract(path)?;
        emit(path, &data)?;
        Ok(data.len())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn romkit_export() -> i32 {
    with_rom(|rom| rom.export(emit))
}

fn game<'r>(rom: &'r Rom<'static>) -> Result<&'r dyn romkit::Game> {
    rom.game()
        .ok_or_else(|| Error::Unsupported(format!("no extractor for {}", rom.identity())))
}
