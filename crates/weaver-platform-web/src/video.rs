//! Single-slot browser external-image bridge. No decoded-frame queue is retained.

#[cfg(any(test, target_arch = "wasm32"))]
struct VideoSlot<T> {
    source: Option<T>,
    dirty: bool,
    remove: bool,
    device_limit: u32,
    fatal: Option<String>,
}

#[cfg(any(test, target_arch = "wasm32"))]
impl<T> Default for VideoSlot<T> {
    fn default() -> Self {
        Self {
            source: None,
            dirty: false,
            remove: false,
            device_limit: 0,
            fatal: None,
        }
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
impl<T> VideoSlot<T> {
    fn attach(&mut self, source: T, width: u32, height: u32) -> bool {
        if !self.valid_size(width, height) {
            return false;
        }
        self.source = Some(source);
        self.dirty = true;
        true
    }

    fn valid_size(&self, width: u32, height: u32) -> bool {
        weaver_render_wgpu::external_image::valid_external_image_size(
            width,
            height,
            self.device_limit,
        )
    }

    fn mark(&mut self, width: u32, height: u32) -> bool {
        if self.source.is_none() || (width > 0 && height > 0 && !self.valid_size(width, height)) {
            return false;
        }
        // This boolean accepts a notification, not a completed GPU copy. A seek
        // can lose decoded dimensions after the caller's readiness check, and a
        // clean paused slot must retain one retry without another media event.
        self.dirty = true;
        true
    }

    fn take_frame(&mut self) -> Option<&T> {
        if !std::mem::take(&mut self.dirty) {
            return None;
        }
        self.source.as_ref()
    }

    fn defer_frame(&mut self) {
        if self.source.is_some() {
            self.dirty = true;
        }
    }

    fn detach(&mut self) {
        self.source = None;
        self.dirty = false;
        self.remove = true;
    }

    fn fail(&mut self, message: String) {
        self.detach();
        self.device_limit = 0;
        // Preserve only the first error and bound browser-visible error storage.
        if self.fatal.is_none() {
            self.fatal = Some(message.chars().take(1024).collect());
        }
    }
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static VIDEO: std::cell::RefCell<VideoSlot<web_sys::HtmlVideoElement>> =
        std::cell::RefCell::new(VideoSlot::default());
}

/// Attach a metadata-ready source, rejecting unsupported or oversized videos.
#[cfg(target_arch = "wasm32")]
pub fn attach_external_video(video: web_sys::HtmlVideoElement) -> bool {
    if video.ready_state() < 1 {
        return false;
    }
    VIDEO.with(|state| {
        let (width, height) = (video.video_width(), video.video_height());
        state.borrow_mut().attach(video, width, height)
    })
}

/// Accept one coalesced frame notification, retaining it while decode is unready.
#[cfg(target_arch = "wasm32")]
pub fn mark_external_video_frame() -> bool {
    VIDEO.with(|state| {
        let mut state = state.borrow_mut();
        let Some(video) = state.source.as_ref() else {
            return false;
        };
        let (width, height) = (video.video_width(), video.video_height());
        state.mark(width, height)
    })
}

/// Release the DOM reference now; remove the GPU override before the next render.
#[cfg(target_arch = "wasm32")]
pub fn detach_external_video() {
    VIDEO.with(|state| state.borrow_mut().detach());
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn initialize(renderer: &weaver_render_wgpu::WgpuRenderer, has_target: bool) {
    VIDEO.with(|state| {
        let mut state = state.borrow_mut();
        state.detach();
        state.fatal = None;
        state.device_limit = if has_target && renderer.supports_external_images() {
            renderer.context().device.limits().max_texture_dimension_2d
        } else {
            0
        };
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn fail(message: String) {
    VIDEO.with(|state| state.borrow_mut().fail(message));
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn reset() {
    VIDEO.with(|state| {
        let mut state = state.borrow_mut();
        state.detach();
        state.device_limit = 0;
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn before_render(
    renderer: &mut weaver_render_wgpu::WgpuRenderer,
    target: Option<weaver_render::SpriteHandle>,
) -> Result<(), String> {
    // Do not hold a RefCell borrow across GPU calls: validation callbacks may
    // synchronously write the bounded fatal signal.
    let (remove, frame) = VIDEO.with(|state| {
        let mut state = state.borrow_mut();
        let remove = std::mem::take(&mut state.remove);
        // Metadata may arrive before the first decoded frame. Keep one pending
        // flag until it is copyable, rather than allocating an empty video image.
        let frame = if state
            .source
            .as_ref()
            .is_some_and(|video| video.ready_state() >= 2)
        {
            state.take_frame().cloned()
        } else {
            None
        };
        (remove, frame)
    });
    if let Some(target) = target {
        if remove {
            renderer.remove_texture_override(target);
        }
        if let Some(video) = frame {
            match renderer.update_external_video(target, &video) {
                Ok(true) => {}
                Ok(false) => VIDEO.with(|state| state.borrow_mut().defer_frame()),
                Err(error) => {
                    renderer.remove_texture_override(target);
                    fail(error.to_string());
                }
            }
        }
    }
    VIDEO.with(|state| state.borrow_mut().fatal.take().map_or(Ok(()), Err))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_slot() -> VideoSlot<u32> {
        VideoSlot {
            device_limit: 4096,
            ..Default::default()
        }
    }

    #[test]
    fn rejects_before_ready_and_invalid_dimensions_without_replacing_source() {
        let mut slot = VideoSlot::<u32>::default();
        assert!(!slot.attach(1, 1920, 1080));
        slot.device_limit = 4096;
        assert!(slot.attach(1, 1920, 1080));
        assert!(!slot.attach(2, 3840, 2160));
        assert_eq!(slot.take_frame(), Some(&1));
        assert!(!slot.attach(2, 0, 1));
        assert!(!slot.mark(3840, 2160));
    }

    #[test]
    fn frames_and_replacement_coalesce_to_one_slot() {
        let mut slot = ready_slot();
        assert!(slot.attach(1, 1280, 720));
        for _ in 0..10_000 {
            assert!(slot.mark(1280, 720));
        }
        assert!(slot.attach(2, 1280, 720));
        assert_eq!(slot.take_frame(), Some(&2));
        assert_eq!(slot.take_frame(), None);
        assert!(slot.mark(1280, 720));
        assert_eq!(slot.take_frame(), Some(&2));
    }

    #[test]
    fn clean_slot_accepts_transient_notifications_and_retries_without_another_event() {
        let mut slot = ready_slot();
        assert!(slot.attach(1, 638, 360));
        assert_eq!(slot.take_frame(), Some(&1));
        assert_eq!(slot.take_frame(), None);
        for (width, height) in [(0, 0), (0, 360), (638, 0), (638, 360)] {
            assert!(slot.mark(width, height));
            assert_eq!(slot.take_frame(), Some(&1));
            slot.defer_frame();
            assert_eq!(slot.take_frame(), Some(&1));
            assert_eq!(slot.take_frame(), None);
        }
        assert!(!slot.mark(3840, 2160));
        assert_eq!(slot.take_frame(), None);
        slot.detach();
        assert!(!slot.mark(0, 0));
        assert!(!slot.mark(638, 360));
    }

    #[test]
    fn deferred_decode_retries_one_slot_and_cannot_revive_detached_or_failed_sources() {
        let mut slot = ready_slot();
        assert!(slot.attach(1, 638, 360));
        assert_eq!(slot.take_frame(), Some(&1));
        for _ in 0..10_000 {
            slot.defer_frame();
        }
        assert_eq!(slot.take_frame(), Some(&1));
        assert_eq!(slot.take_frame(), None);
        slot.detach();
        slot.defer_frame();
        assert_eq!(slot.take_frame(), None);
        assert!(slot.attach(2, 638, 360));
        slot.fail("decode failed".into());
        slot.defer_frame();
        assert_eq!(slot.take_frame(), None);
    }

    #[test]
    fn detach_drops_source_cancels_frame_and_requests_removal() {
        use std::rc::Rc;
        let source = Rc::new(());
        let mut slot = VideoSlot {
            device_limit: 4096,
            ..Default::default()
        };
        assert!(slot.attach(source.clone(), 1, 1));
        assert_eq!(Rc::strong_count(&source), 2);
        slot.detach();
        assert_eq!(Rc::strong_count(&source), 1);
        assert_eq!(slot.take_frame(), None);
        assert!(!slot.mark(1, 1));
        assert!(std::mem::take(&mut slot.remove));
        assert!(!std::mem::take(&mut slot.remove));
        slot.detach();
        assert!(slot.attach(source, 1, 1));
        assert!(slot.remove); // Reattach cannot swallow pending resource removal.
    }

    #[test]
    fn failure_is_bounded_disables_attach_and_cancels_pending_work() {
        let mut slot = ready_slot();
        assert!(slot.attach(1, 1, 1));
        slot.fail("x".repeat(2000));
        slot.fail("later error".into());
        assert_eq!(slot.fatal.as_ref().unwrap().len(), 1024);
        assert!(!slot.attach(2, 1, 1));
        assert_eq!(slot.take_frame(), None);
        assert!(slot.remove);
    }
}
