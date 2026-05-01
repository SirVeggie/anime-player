use std::mem::size_of;
use std::path::Path;

use base64::Engine;
use windows::core::HSTRING;
use windows::Win32::Foundation::{RPC_E_CHANGED_MODE, SIZE};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_THUMBNAILONLY,
};

struct ComApartment {
    should_uninitialize: bool,
}

impl ComApartment {
    fn initialize() -> Result<Self, String> {
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result.is_ok() {
            return Ok(Self {
                should_uninitialize: true,
            });
        }

        if result == RPC_E_CHANGED_MODE {
            return Ok(Self {
                should_uninitialize: false,
            });
        }

        Err(format!("failed to initialize COM: {result:?}"))
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.should_uninitialize {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

struct BitmapHandle(HBITMAP);

impl Drop for BitmapHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0 .0));
        }
    }
}

#[tauri::command]
pub async fn get_file_thumbnail(path: String, size: u32) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || get_file_thumbnail_blocking(path, size))
        .await
        .map_err(|e| e.to_string())?
}

fn get_file_thumbnail_blocking(path: String, size: u32) -> Result<Option<String>, String> {
    if !Path::new(&path).is_file() {
        return Ok(None);
    }

    let _com = ComApartment::initialize()?;
    let size = size.clamp(32, 512) as i32;
    let item: IShellItemImageFactory = unsafe {
        match SHCreateItemFromParsingName(&HSTRING::from(path), None) {
            Ok(item) => item,
            Err(_) => return Ok(None),
        }
    };

    let bitmap = unsafe {
        match item.GetImage(
            SIZE { cx: size, cy: size },
            SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK,
        ) {
            Ok(bitmap) => BitmapHandle(bitmap),
            Err(_) => return Ok(None),
        }
    };

    bitmap_to_bmp_data_url(bitmap.0).map(Some)
}

fn bitmap_to_bmp_data_url(bitmap: HBITMAP) -> Result<String, String> {
    let mut bitmap_info = BITMAP::default();
    let object_size = size_of::<BITMAP>() as i32;
    let object_result = unsafe {
        GetObjectW(
            HGDIOBJ(bitmap.0),
            object_size,
            Some((&mut bitmap_info as *mut BITMAP).cast()),
        )
    };
    if object_result != object_size {
        return Err("failed to read thumbnail bitmap metadata".to_string());
    }

    let width = bitmap_info.bmWidth.max(1);
    let height = bitmap_info.bmHeight.abs().max(1);
    let stride = width as usize * 4;
    let image_size = stride
        .checked_mul(height as usize)
        .ok_or("thumbnail bitmap is too large")?;
    let mut pixels = vec![0_u8; image_size];
    let mut dib_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: image_size as u32,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        ..BITMAPINFO::default()
    };

    let hdc = unsafe { GetDC(None) };
    if hdc.0.is_null() {
        return Err("failed to acquire device context for thumbnail".to_string());
    }

    let scan_lines = unsafe {
        GetDIBits(
            hdc,
            bitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut dib_info,
            DIB_RGB_COLORS,
        )
    };
    unsafe {
        let _ = ReleaseDC(None, hdc);
    }

    if scan_lines == 0 {
        return Err("failed to copy thumbnail bitmap pixels".to_string());
    }

    let mut bmp = Vec::with_capacity(14 + size_of::<BITMAPINFOHEADER>() + pixels.len());
    let file_size = (14 + size_of::<BITMAPINFOHEADER>() + pixels.len()) as u32;
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&file_size.to_le_bytes());
    bmp.extend_from_slice(&0_u16.to_le_bytes());
    bmp.extend_from_slice(&0_u16.to_le_bytes());
    bmp.extend_from_slice(&(14_u32 + size_of::<BITMAPINFOHEADER>() as u32).to_le_bytes());
    bmp.extend_from_slice(&dib_info.bmiHeader.biSize.to_le_bytes());
    bmp.extend_from_slice(&dib_info.bmiHeader.biWidth.to_le_bytes());
    bmp.extend_from_slice(&dib_info.bmiHeader.biHeight.to_le_bytes());
    bmp.extend_from_slice(&dib_info.bmiHeader.biPlanes.to_le_bytes());
    bmp.extend_from_slice(&dib_info.bmiHeader.biBitCount.to_le_bytes());
    bmp.extend_from_slice(&dib_info.bmiHeader.biCompression.to_le_bytes());
    bmp.extend_from_slice(&dib_info.bmiHeader.biSizeImage.to_le_bytes());
    bmp.extend_from_slice(&dib_info.bmiHeader.biXPelsPerMeter.to_le_bytes());
    bmp.extend_from_slice(&dib_info.bmiHeader.biYPelsPerMeter.to_le_bytes());
    bmp.extend_from_slice(&dib_info.bmiHeader.biClrUsed.to_le_bytes());
    bmp.extend_from_slice(&dib_info.bmiHeader.biClrImportant.to_le_bytes());
    bmp.extend_from_slice(&pixels);

    let encoded = base64::engine::general_purpose::STANDARD.encode(bmp);
    Ok(format!("data:image/bmp;base64,{encoded}"))
}
