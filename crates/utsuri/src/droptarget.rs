//! The radial menu's OLE drop target. Rune registers one for its windows only, and the menu
//! is an overlay (it must never take focus), so files dragged onto it would fall through to
//! the app behind. This finds the menu's window on the UI thread and registers a target
//! that tells the hub about the drag and the drop.

use crate::Hub;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use windows::Win32::Foundation::{HWND, LPARAM, POINTL};
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl, OleInitialize, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{EnumThreadWindows, GetWindowTextW};
use windows::core::{BOOL, Ref, implement};

thread_local! {
    static DONE: RefCell<HashSet<isize>> = RefCell::new(HashSet::new());
}

unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
    let (title, out) = unsafe { &mut *(lp.0 as *mut (&str, Vec<isize>)) };
    let mut buf = [0u16; 128];
    let n = unsafe { GetWindowTextW(h, &mut buf) } as usize;
    if String::from_utf16_lossy(&buf[..n]) == *title {
        out.push(h.0 as isize);
    }
    BOOL(1)
}

/// Register the target on this thread's window titled `title` (the overlay's name), if it
/// exists and hasn't got one. Call on the UI thread; cheap to repeat.
pub fn ensure(title: &str, app: &rune::AppHandle<Hub>) {
    let mut found: (&str, Vec<isize>) = (title, vec![]);
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(each), LPARAM(&mut found as *mut _ as isize));
    }
    for h in found.1 {
        if DONE.with(|d| d.borrow().contains(&h)) {
            continue;
        }
        unsafe {
            if OleInitialize(None).is_err() {
                return;
            }
            let target: IDropTarget = Target { app: app.clone(), files: std::cell::Cell::new(false) }.into();
            // the overlay carries winit's target, whose events Rune's overlays ignore
            let _ = RevokeDragDrop(HWND(h as *mut _));
            match RegisterDragDrop(HWND(h as *mut _), &target) {
                Ok(()) => log::debug!("menu takes drops (window {h:#x})"),
                Err(e) => log::debug!("menu drop target: {e}"),
            }
        }
        DONE.with(|d| d.borrow_mut().insert(h));
    }
}

#[implement(IDropTarget)]
struct Target {
    app: rune::AppHandle<Hub>,
    files: std::cell::Cell<bool>,
}

/// The paths `data` carries as a shell file list (CF_HDROP).
fn paths(data: Option<&IDataObject>) -> Vec<PathBuf> {
    let Some(data) = data else { return vec![] };
    let format = FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
    unsafe {
        let Ok(mut medium) = data.GetData(&format) else { return vec![] };
        let drop = HDROP(medium.u.hGlobal.0);
        let count = DragQueryFileW(drop, u32::MAX, None);
        let out = (0..count)
            .filter_map(|i| {
                let len = DragQueryFileW(drop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                (DragQueryFileW(drop, i, Some(&mut buf)) as usize == len).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len])))
            })
            .collect();
        ReleaseStgMedium(&mut medium);
        out
    }
}

fn effect(out: *mut DROPEFFECT, files: bool) {
    if !out.is_null() {
        unsafe { *out = if files { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, out: *mut DROPEFFECT) -> windows::core::Result<()> {
        let files = paths(data.as_ref());
        self.files.set(!files.is_empty());
        effect(out, !files.is_empty());
        if !files.is_empty() {
            log::debug!("drag over the menu: {}", files[0].display());
            self.app.update(move |h, _| h.menu_drag_entered(files));
        }
        Ok(())
    }

    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, _: &POINTL, out: *mut DROPEFFECT) -> windows::core::Result<()> {
        effect(out, self.files.get());
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.files.set(false);
        Ok(())
    }

    fn Drop(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, out: *mut DROPEFFECT) -> windows::core::Result<()> {
        let files = paths(data.as_ref());
        self.files.set(false);
        effect(out, !files.is_empty());
        if !files.is_empty() {
            for f in &files {
                log::debug!("dropped: {}", f.display());
            }
            self.app.update(move |h, tx| h.menu_dropped(files, tx));
        }
        Ok(())
    }
}
