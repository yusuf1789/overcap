use anyhow::{anyhow, Result};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use windows::core::Interface;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BOX,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic, IsWindow};

use crate::capture::CapturedFrame;
use crate::config::Region;

/// Captures a specific target window directly from the GPU via Windows Graphics Capture (WGC).
/// Because only the target game/app window is captured, the Overcap overlay is never included
/// in the captured frames even when `WDA_EXCLUDEFROMCAPTURE` is disabled for Stream Mode.
pub struct WindowCapturer {
    d3d_device: ID3D11Device,
    d3d_context: ID3D11DeviceContext,
    winrt_device: IDirect3DDevice,
    target_hwnd: Option<isize>,
    item: Option<GraphicsCaptureItem>,
    frame_pool: Option<Direct3D11CaptureFramePool>,
    session: Option<GraphicsCaptureSession>,
    pool_size: SizeInt32,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
}

impl WindowCapturer {
    pub fn new() -> Result<Self> {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let feature_levels = [D3D_FEATURE_LEVEL_11_0];

        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                None,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }

        let d3d_device = device.ok_or_else(|| anyhow!("D3D11CreateDevice returned no device"))?;
        let d3d_context = context.ok_or_else(|| anyhow!("D3D11CreateDevice returned no context"))?;
        let dxgi_device: IDXGIDevice = d3d_device.cast()?;
        let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device)? };
        let winrt_device: IDirect3DDevice = inspectable.cast()?;

        Ok(Self {
            d3d_device,
            d3d_context,
            winrt_device,
            target_hwnd: None,
            item: None,
            frame_pool: None,
            session: None,
            pool_size: SizeInt32 {
                Width: 0,
                Height: 0,
            },
            staging: None,
        })
    }

    pub fn reset(&mut self) {
        if let Some(session) = self.session.take() {
            let _ = session.Close();
        }
        if let Some(pool) = self.frame_pool.take() {
            let _ = pool.Close();
        }
        self.item = None;
        self.target_hwnd = None;
        self.pool_size = SizeInt32 {
            Width: 0,
            Height: 0,
        };
    }

    fn attach_window(&mut self, hwnd: HWND) -> Result<()> {
        self.reset();

        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd)? };
        let size = item.Size()?;
        if size.Width <= 0 || size.Height <= 0 {
            return Err(anyhow!("Target window has empty capture size"));
        }

        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &self.winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            1,
            size,
        )?;
        let session = frame_pool.CreateCaptureSession(&item)?;
        let _ = session.SetIsCursorCaptureEnabled(false);
        // Hide the yellow capture border on Windows 11 (best-effort; ignored on older builds).
        let _ = session.SetIsBorderRequired(false);
        session.StartCapture()?;

        self.target_hwnd = Some(hwnd.0 as isize);
        self.item = Some(item);
        self.frame_pool = Some(frame_pool);
        self.session = Some(session);
        self.pool_size = size;
        Ok(())
    }

    fn ensure_staging(&mut self, width: u32, height: u32) -> Result<&ID3D11Texture2D> {
        let needs_new = match &self.staging {
            Some((_, w, h)) => *w != width || *h != height,
            None => true,
        };

        if needs_new {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut tex: Option<ID3D11Texture2D> = None;
            unsafe {
                self.d3d_device
                    .CreateTexture2D(&desc, None, Some(&mut tex))?;
            }
            let tex = tex.ok_or_else(|| anyhow!("CreateTexture2D returned no staging texture"))?;
            self.staging = Some((tex, width, height));
        }

        Ok(&self.staging.as_ref().unwrap().0)
    }

    /// Captures the screen-space `region` cropped from `hwnd`.
    /// Returns `Ok(None)` if no new frame is ready yet or the region does not intersect the window.
    pub fn capture_window_region(
        &mut self,
        hwnd: HWND,
        region: Region,
    ) -> Result<Option<CapturedFrame>> {
        if region.width <= 0 || region.height <= 0 {
            return Ok(None);
        }

        unsafe {
            if !IsWindow(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                return Ok(None);
            }
        }

        if self.target_hwnd != Some(hwnd.0 as isize) {
            self.attach_window(hwnd)?;
        }

        let Some(pool) = self.frame_pool.as_ref() else {
            return Ok(None);
        };

        // Drain to the freshest available frame in the pool.
        let mut latest_frame = None;
        while let Ok(frame) = pool.TryGetNextFrame() {
            if let Some(old) = latest_frame.replace(frame) {
                let _ = old.Close();
            }
        }

        let Some(frame) = latest_frame else {
            return Ok(None);
        };

        let content_size = frame.ContentSize()?;
        if content_size.Width <= 0 || content_size.Height <= 0 {
            let _ = frame.Close();
            return Ok(None);
        }

        // Resize frame pool if the target window resized.
        if content_size.Width != self.pool_size.Width || content_size.Height != self.pool_size.Height
        {
            if let Some(pool) = self.frame_pool.as_ref() {
                let _ = pool.Recreate(
                    &self.winrt_device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    1,
                    content_size,
                );
                self.pool_size = content_size;
            }
        }

        let surface = frame.Surface()?;
        let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
        let src_tex: ID3D11Texture2D = unsafe { access.GetInterface()? };

        let mut src_desc = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            src_tex.GetDesc(&mut src_desc);
        }

        let win_rect = window_bounds(hwnd)?;
        let win_w = (win_rect.right - win_rect.left).max(1);
        let win_h = (win_rect.bottom - win_rect.top).max(1);

        // Intersect requested screen-space region with the target window bounds.
        let inter_left = region.left.max(win_rect.left);
        let inter_top = region.top.max(win_rect.top);
        let inter_right = (region.left + region.width).min(win_rect.right);
        let inter_bottom = (region.top + region.height).min(win_rect.bottom);

        if inter_right <= inter_left || inter_bottom <= inter_top {
            let _ = frame.Close();
            return Ok(None);
        }

        // Map screen-space intersection into texture coordinates.
        let tex_w = src_desc.Width as i32;
        let tex_h = src_desc.Height as i32;
        let sx = (((inter_left - win_rect.left) as i64 * tex_w as i64) / win_w as i64)
            .clamp(0, (tex_w - 1).max(0) as i64) as u32;
        let sy = (((inter_top - win_rect.top) as i64 * tex_h as i64) / win_h as i64)
            .clamp(0, (tex_h - 1).max(0) as i64) as u32;
        let ex = (((inter_right - win_rect.left) as i64 * tex_w as i64) / win_w as i64)
            .clamp((sx + 1) as i64, tex_w as i64) as u32;
        let ey = (((inter_bottom - win_rect.top) as i64 * tex_h as i64) / win_h as i64)
            .clamp((sy + 1) as i64, tex_h as i64) as u32;

        let crop_w = ex.saturating_sub(sx);
        let crop_h = ey.saturating_sub(sy);
        if crop_w < 8 || crop_h < 8 {
            let _ = frame.Close();
            return Ok(None);
        }

        let staging_tex = self.ensure_staging(crop_w, crop_h)?.clone();
        let src_box = D3D11_BOX {
            left: sx,
            top: sy,
            front: 0,
            right: ex,
            bottom: ey,
            back: 1,
        };

        let mut bgra = vec![0u8; (crop_w as usize) * (crop_h as usize) * 4];
        unsafe {
            self.d3d_context.CopySubresourceRegion(
                &staging_tex,
                0,
                0,
                0,
                0,
                &src_tex,
                0,
                Some(&src_box),
            );

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.d3d_context
                .Map(&staging_tex, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;

            let row_bytes = (crop_w as usize) * 4;
            let src_pitch = mapped.RowPitch as usize;
            let src_base = mapped.pData as *const u8;

            for row in 0..(crop_h as usize) {
                ptr::copy_nonoverlapping(
                    src_base.add(row * src_pitch),
                    bgra.as_mut_ptr().add(row * row_bytes),
                    row_bytes,
                );
            }

            self.d3d_context.Unmap(&staging_tex, 0);
        }

        let _ = frame.Close();

        Ok(Some(CapturedFrame {
            width: crop_w as i32,
            height: crop_h as i32,
            bgra,
        }))
    }
}

impl Drop for WindowCapturer {
    fn drop(&mut self) {
        self.reset();
    }
}

fn window_bounds(hwnd: HWND) -> Result<RECT> {
    let mut rect = RECT::default();
    unsafe {
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut c_void,
            size_of::<RECT>() as u32,
        )
        .is_ok()
        {
            return Ok(rect);
        }
        GetWindowRect(hwnd, &mut rect)?;
    }
    Ok(rect)
}
