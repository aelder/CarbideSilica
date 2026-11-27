//! The bundled fonts. Futura is an Apple font and cannot ship on Linux, so the app carries
//! open fonts in `linux/resources/fonts` (each with its SIL Open Font License):
//!
//! - Jost (SIL OFL 1.1), a geometric face after Futura, for the Futura roles.
//! - Inter (SIL OFL 1.1) for sentences, in place of the macOS system face.
//! - IBM Plex Mono (SIL OFL 1.1) for IDs and hashes, in place of the macOS monospaced face.
//!
//! The fonts are given to fontconfig as application fonts before GTK starts. They are not
//! installed on the system. They are added to the configuration that fontconfig loaded, so
//! the fonts of the system stay in it: Pango takes a glyph that Jost, Inter and Plex Mono do
//! not have (an emoji, a Chinese or Arabic letter) from a system font, such as Noto Color
//! Emoji and Noto Sans CJK.

use std::ffi::CString;
use std::path::PathBuf;

#[link(name = "fontconfig")]
unsafe extern "C" {
    fn FcConfigAppFontAddDir(config: *mut std::ffi::c_void, dir: *const std::os::raw::c_char) -> i32;
}

/// The folder of the app's resources: `CARBIDE_SILICA_RESOURCES`, or `resources` beside the
/// installed binary (`<prefix>/share/carbide-silica`), or the folder of the source tree.
pub fn resources() -> PathBuf {
    if let Some(path) = std::env::var_os("CARBIDE_SILICA_RESOURCES") {
        return PathBuf::from(path);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(prefix) = exe.parent().and_then(|bin| bin.parent())
    {
        let installed = prefix.join("share/carbide-silica");
        if installed.is_dir() {
            return installed;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources")
}

/// Adds the bundled fonts to fontconfig for this process. Call before GTK starts.
pub fn register() -> bool {
    let folder = resources().join("fonts");
    let Ok(path) = CString::new(folder.to_string_lossy().as_bytes()) else { return false };
    // SAFETY: a null configuration means the current one. The path is a valid C string.
    unsafe { FcConfigAppFontAddDir(std::ptr::null_mut(), path.as_ptr()) != 0 }
}
