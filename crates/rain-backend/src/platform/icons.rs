//! Icons as PNG: of a file, of a shortcut and of an app id. Foundation 11.6 and 4.7.
//!
//! Windows draws the icon through `IShellItemImageFactory::GetImage` (the shell's own image
//! cache), asking for the icon and never for a thumbnail or a preview. The bitmap is turned into
//! straight (not premultiplied) RGBA and encoded with the `png` crate. Measured in the spike
//! `winapi`: the first call 40 to 65 ms, a warm one 1 to 11 ms, a 64 px icon about 170 bytes to
//! a few KB. Every call runs on the COM thread.

use std::path::Path;

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, HGDIOBJ, ReleaseDC,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows::core::PCWSTR;

use super::com::{self, ComError};

/// Where the icon comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconSource<'a> {
    /// A program or any file: its icon.
    File(&'a Path),
    /// A shortcut (`.lnk`): the icon it shows, which may differ from the target's.
    Shortcut(&'a Path),
    /// An app id (an AUMID) from the app list: a packaged app or any item of `shell:AppsFolder`.
    AppId(&'a str),
}

impl IconSource<'_> {
    /// The parsing name that the shell understands.
    fn parsing_name(&self) -> String {
        match self {
            Self::File(path) | Self::Shortcut(path) => path.to_string_lossy().into_owned(),
            Self::AppId(aumid) => format!(r"shell:AppsFolder\{aumid}"),
        }
    }
}

/// The icon as a PNG of `size` x `size` pixels (the shell may return a larger one scaled down).
pub async fn icon_png(source: IconSource<'_>, size: u32) -> Result<Vec<u8>, ComError> {
    let name = source.parsing_name();
    com::run(move || render(&name, size)).await
}

/// Draws the icon. Must run on the COM thread.
pub fn render(parsing_name: &str, size: u32) -> Result<Vec<u8>, ComError> {
    let wide: Vec<u16> = parsing_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)? };
    let edge = i32::try_from(size).unwrap_or(64);
    // SAFETY: `factory` is a valid interface; the flags ask for an icon, scaled down if bigger.
    let bitmap = unsafe {
        factory.GetImage(
            SIZE { cx: edge, cy: edge },
            SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
        )?
    };
    let image = read_pixels(bitmap);
    // SAFETY: the bitmap belongs to this call (the shell handed it over) and is not used again.
    unsafe {
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
    }
    let image = image?;
    encode_png(&image).map_err(|_| ComError::Os { code: -1 })
}

/// An image as straight RGBA, rows from the top.
struct Rgba {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

fn read_pixels(bitmap: HBITMAP) -> Result<Rgba, ComError> {
    let mut info = BITMAP::default();
    // SAFETY: `info` is a valid `BITMAP` and the size passed is its size.
    let described = unsafe {
        GetObjectW(
            HGDIOBJ(bitmap.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(std::ptr::from_mut(&mut info).cast()),
        )
    };
    if described == 0 {
        return Err(ComError::Os { code: -1 });
    }
    let (width, height) = (info.bmWidth as u32, info.bmHeight.unsigned_abs());
    let mut header = BITMAPINFO::default();
    header.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    header.bmiHeader.biWidth = width as i32;
    header.bmiHeader.biHeight = -(height as i32); // negative: rows from the top
    header.bmiHeader.biPlanes = 1;
    header.bmiHeader.biBitCount = 32;
    header.bmiHeader.biCompression = BI_RGB.0;

    let mut pixels = vec![0u8; (width * height * 4) as usize];
    // SAFETY: the screen device context is released below; `pixels` holds exactly
    // `width * height * 4` bytes, which is what 32 bits per pixel over `height` lines needs.
    let lines = unsafe {
        let screen = GetDC(None);
        let lines = GetDIBits(
            screen,
            bitmap,
            0,
            height,
            Some(pixels.as_mut_ptr().cast()),
            &mut header,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, screen);
        lines
    };
    if lines as u32 != height {
        return Err(ComError::Os { code: -1 });
    }
    to_straight_rgba(&mut pixels);
    Ok(Rgba {
        width,
        height,
        pixels,
    })
}

/// BGRA from Windows to straight RGBA. Handles an icon without alpha (all alpha 0: made opaque)
/// and premultiplied alpha (every colour at most its alpha: un-premultiplied).
fn to_straight_rgba(pixels: &mut [u8]) {
    let (pixels, _) = pixels.as_chunks_mut::<4>();
    let has_alpha = pixels.iter().any(|p| p[3] != 0);
    let premultiplied = pixels
        .iter()
        .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]);
    for p in pixels {
        p.swap(0, 2); // BGRA to RGBA
        if !has_alpha {
            p[3] = 255;
        } else if premultiplied && p[3] != 0 && p[3] != 255 {
            let alpha = u32::from(p[3]);
            for channel in &mut p[0..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}

fn encode_png(image: &Rgba) -> Result<Vec<u8>, png::EncodingError> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.write_header()?.write_image_data(&image.pixels)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    fn system_file(relative: &str) -> PathBuf {
        PathBuf::from(std::env::var("SystemRoot").unwrap()).join(relative)
    }

    /// Reads the size of a PNG from its header.
    fn dimensions(png: &[u8]) -> (u32, u32) {
        let number = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
        (number(16), number(20))
    }

    #[tokio::test]
    async fn a_program_has_a_png_icon_of_the_asked_size() {
        let notepad = system_file(r"System32\notepad.exe");
        let png = icon_png(IconSource::File(&notepad), 64).await.unwrap();
        assert_eq!(&png[..8], &PNG_SIGNATURE);
        assert_eq!(dimensions(&png), (64, 64));
        assert!(png.len() > 100);
    }

    #[tokio::test]
    async fn a_shortcut_has_an_icon_too() {
        // A shortcut that this test makes itself, so no installed program is needed.
        let temp = tempfile::tempdir().unwrap();
        let link = temp.path().join("Test.lnk");
        let target = system_file(r"System32\notepad.exe");
        super::super::shell::write_shortcut_for_tests(&link, &target).await;

        let png = icon_png(IconSource::Shortcut(&link), 48).await.unwrap();
        assert_eq!(&png[..8], &PNG_SIGNATURE);
        assert_eq!(dimensions(&png), (48, 48));
    }

    #[tokio::test]
    async fn an_app_id_of_the_apps_folder_has_an_icon() {
        // The first item of the apps folder that renders, whichever apps this PC has.
        let apps = com::run(super::super::shell::list_apps).await.unwrap();
        for app in apps.iter().take(20) {
            if let Ok(png) = icon_png(IconSource::AppId(&app.aumid), 64).await {
                assert_eq!(&png[..8], &PNG_SIGNATURE);
                return;
            }
        }
        panic!("no app of the apps folder had an icon");
    }

    #[tokio::test]
    async fn a_file_that_does_not_exist_gives_an_error_and_the_thread_goes_on() {
        let missing = system_file(r"System32\no-such-program-rain-test.exe");
        assert!(icon_png(IconSource::File(&missing), 64).await.is_err());
        let notepad = system_file(r"System32\notepad.exe");
        assert!(icon_png(IconSource::File(&notepad), 64).await.is_ok());
    }

    #[test]
    fn an_icon_without_alpha_becomes_opaque_and_blue_and_red_swap() {
        let mut pixels = vec![10, 20, 30, 0, 1, 2, 3, 0]; // BGRA, alpha unused
        to_straight_rgba(&mut pixels);
        assert_eq!(pixels, [30, 20, 10, 255, 3, 2, 1, 255]);
    }

    #[test]
    fn premultiplied_alpha_is_undone_and_straight_alpha_is_kept() {
        // Premultiplied: every colour is at most its alpha.
        let mut premultiplied = vec![50, 50, 50, 100];
        to_straight_rgba(&mut premultiplied);
        assert_eq!(premultiplied, [128, 128, 128, 100]);
        // Straight: a colour above its alpha cannot be premultiplied, so it is left alone.
        let mut straight = vec![200, 100, 50, 100];
        to_straight_rgba(&mut straight);
        assert_eq!(straight, [50, 100, 200, 100]);
    }

    #[test]
    fn a_png_is_written_for_a_small_image() {
        let image = Rgba {
            width: 2,
            height: 2,
            pixels: vec![255; 16],
        };
        let png = encode_png(&image).unwrap();
        assert_eq!(&png[..8], &PNG_SIGNATURE);
        assert_eq!(dimensions(&png), (2, 2));
    }
}
