//! Capture the screenshot frame even when its render target has no swapchain.

use std::ffi::{CStr, c_int, c_void};
use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::{Arc, OnceLock};

use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::{Render, RenderApp, RenderSystems};
use libloading::os::windows::Library;
use renderdoc_sys::{RENDERDOC_API_1_6_0, RENDERDOC_Version, eRENDERDOC_API_Version_1_6_0};

#[derive(Clone, Default, ExtractResource, Resource)]
pub(super) struct Request {
    pub armed: bool,
    pub outcome: Arc<OnceLock<Result<PathBuf, String>>>,
}

#[derive(Default, Resource)]
struct CaptureState(Option<ActiveCapture>);

struct ActiveCapture {
    _library: Library,
    api: RENDERDOC_API_1_6_0,
    captures_before: u32,
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<Request>()
        .add_plugins(ExtractResourcePlugin::<Request>::default());
    app.sub_app_mut(RenderApp)
        .init_resource::<CaptureState>()
        .add_systems(
            Render,
            (
                begin
                    .after(RenderSystems::Prepare)
                    .before(RenderSystems::Render),
                finish
                    .after(RenderSystems::Render)
                    .before(RenderSystems::Cleanup),
            ),
        );
}

fn begin(request: Option<Res<Request>>, mut state: ResMut<CaptureState>) {
    let Some(request) = request.filter(|request| request.armed && request.outcome.get().is_none())
    else {
        return;
    };
    if state.0.is_some() {
        return;
    }
    match ActiveCapture::start() {
        Ok(capture) => state.0 = Some(capture),
        Err(error) => {
            let _ = request.outcome.set(Err(error));
        }
    }
}

fn finish(request: Option<Res<Request>>, mut state: ResMut<CaptureState>) {
    if let Some(capture) = state.0.take() {
        let result = capture.stop();
        if let Some(request) = request {
            let _ = request.outcome.set(result);
        }
    }
}

impl ActiveCapture {
    fn start() -> Result<Self, String> {
        let library = Library::open_already_loaded("renderdoc.dll")
            .map_err(|_| "launch the renderer under RenderDoc before using --renderdoc")?;
        type GetApi = unsafe extern "C" fn(RENDERDOC_Version, *mut *mut c_void) -> c_int;
        // RenderDoc exports this entry point with the C ABI for the lifetime of the DLL.
        let get_api = unsafe { library.get::<GetApi>(b"RENDERDOC_GetAPI\0") }
            .map_err(|e| format!("resolve RENDERDOC_GetAPI: {e}"))?;
        let mut table = null_mut();
        // A successful query supplies the API table for this exact version.
        if unsafe { get_api(eRENDERDOC_API_Version_1_6_0, &mut table) } != 1 || table.is_null() {
            return Err("RenderDoc does not expose the 1.6 application API".into());
        }
        let api = unsafe { *table.cast::<RENDERDOC_API_1_6_0>() };
        let (Some(start), Some(is_capturing), Some(count)) =
            (api.StartFrameCapture, api.IsFrameCapturing, api.GetNumCaptures)
        else {
            return Err("RenderDoc application API is incomplete".into());
        };
        if api.EndFrameCapture.is_none() || api.GetCapture.is_none() {
            return Err("RenderDoc application API is incomplete".into());
        }
        // Null device and window select the current capture target, including offscreen draws.
        let captures_before = unsafe {
            if is_capturing() != 0 {
                return Err("another RenderDoc capture is already active".into());
            }
            let before = count();
            start(null_mut(), null_mut());
            if is_capturing() == 0 {
                return Err("RenderDoc could not begin capturing the GPU device".into());
            }
            before
        };
        Ok(Self {
            _library: library,
            api,
            captures_before,
        })
    }

    fn stop(self) -> Result<PathBuf, String> {
        let end = self.api.EndFrameCapture.unwrap();
        let count = self.api.GetNumCaptures.unwrap();
        let get_capture = self.api.GetCapture.unwrap();
        // The API and its returned indices remain valid until the retained DLL is released.
        unsafe {
            if end(null_mut(), null_mut()) == 0 || count() <= self.captures_before {
                return Err("RenderDoc did not save the offscreen frame".into());
            }
            let mut len = 0;
            let index = self.captures_before;
            if get_capture(index, null_mut(), &mut len, null_mut()) == 0 || len == 0 {
                return Err("RenderDoc did not report the capture path".into());
            }
            let mut bytes = vec![0; len as usize];
            if get_capture(index, bytes.as_mut_ptr().cast(), &mut len, null_mut()) == 0 {
                return Err("could not read the capture path from RenderDoc".into());
            }
            let path = CStr::from_bytes_until_nul(&bytes)
                .map_err(|_| "RenderDoc returned an invalid capture path")?
                .to_str()
                .map_err(|_| "RenderDoc returned a non-UTF-8 capture path")?;
            Ok(path.into())
        }
    }
}
