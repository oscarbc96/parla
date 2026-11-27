//! Audio device changes, as events: a new default microphone or speaker
//! (AirPods connecting) or a device plugged in or out.
//!
//! Event-driven, never polled. On macOS it registers CoreAudio property
//! listeners on the system object for the default input, the default output
//! and the device list; CoreAudio calls them on its own thread when one
//! changes. Elsewhere it is a stub that never fires (the owner still reopens
//! on a stream error). A watcher exists only while a call runs.

use std::sync::Arc;

/// What changed. Several flags may be set at once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeviceChange {
    /// The system default microphone changed.
    pub default_input: bool,
    /// The system default speaker changed.
    pub default_output: bool,
    /// A device was connected or disconnected.
    pub list: bool,
}

/// Called on a platform thread; must not block.
pub type OnDeviceChange = Arc<dyn Fn(DeviceChange) + Send + Sync>;

/// Watches the audio devices until dropped.
pub struct DeviceWatcher {
    #[cfg(target_os = "macos")]
    _inner: mac::Listeners,
}

impl DeviceWatcher {
    /// Starts watching; `on_change` runs on every change.
    pub fn start(on_change: OnDeviceChange) -> Self {
        #[cfg(target_os = "macos")]
        {
            Self {
                _inner: mac::Listeners::add(on_change),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = on_change;
            Self {}
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{DeviceChange, OnDeviceChange};
    use block2::RcBlock;
    use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
    use objc2_core_audio::{
        AudioObjectAddPropertyListenerBlock, AudioObjectPropertyAddress,
        AudioObjectRemovePropertyListenerBlock, kAudioHardwarePropertyDefaultInputDevice,
        kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices,
        kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
    };
    use std::ptr::NonNull;

    const SELECTORS: [u32; 3] = [
        kAudioHardwarePropertyDefaultInputDevice,
        kAudioHardwarePropertyDefaultOutputDevice,
        kAudioHardwarePropertyDevices,
    ];

    fn address(selector: u32) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        }
    }

    type Listener = RcBlock<dyn Fn(u32, NonNull<AudioObjectPropertyAddress>)>;

    /// The registered listener blocks and the private serial queue they run
    /// on. Memory safety needs no proof about in-flight callbacks: each block
    /// owns its own `Arc` of the callback, and CoreAudio retains a block while
    /// it runs. Dropping first removes the blocks, then waits on the queue
    /// (`exec_sync`), so no callback runs after the watcher is gone.
    pub(super) struct Listeners {
        queue: DispatchRetained<DispatchQueue>,
        added: Vec<(u32, Listener)>,
    }

    // SAFETY: the queue and the blocks are thread-safe CoreFoundation and
    // libdispatch objects; the closure inside each block is `Send + Sync`.
    unsafe impl Send for Listeners {}

    impl Listeners {
        pub(super) fn add(on_change: OnDeviceChange) -> Self {
            let queue = DispatchQueue::new("parla.audio-devices", DispatchQueueAttr::SERIAL);
            let mut added = Vec::new();
            for sel in SELECTORS {
                let cb = on_change.clone();
                let block: Listener = RcBlock::new(
                    move |count: u32, addresses: NonNull<AudioObjectPropertyAddress>| {
                        // SAFETY: CoreAudio passes `count` valid addresses.
                        let addresses = unsafe {
                            std::slice::from_raw_parts(addresses.as_ptr(), count as usize)
                        };
                        let mut change = DeviceChange::default();
                        for a in addresses {
                            let sel = a.mSelector;
                            change.default_input |= sel == kAudioHardwarePropertyDefaultInputDevice;
                            change.default_output |=
                                sel == kAudioHardwarePropertyDefaultOutputDevice;
                            change.list |= sel == kAudioHardwarePropertyDevices;
                        }
                        if change != DeviceChange::default() {
                            cb(change);
                        }
                    },
                );
                let a = address(sel);
                // SAFETY: a valid address, queue and block; CoreAudio
                // retains the block until it is removed.
                let status = unsafe {
                    AudioObjectAddPropertyListenerBlock(
                        kAudioObjectSystemObject as u32,
                        NonNull::from(&a),
                        Some(&queue),
                        RcBlock::as_ptr(&block),
                    )
                };
                if status == 0 {
                    added.push((sel, block));
                } else {
                    tracing::warn!("audio device listener {sel:#x} not added: {status}");
                }
            }
            Self { queue, added }
        }
    }

    impl Drop for Listeners {
        fn drop(&mut self) {
            for (sel, block) in &self.added {
                let a = address(*sel);
                // SAFETY: removes exactly the block `add` registered, with
                // the same queue.
                unsafe {
                    AudioObjectRemovePropertyListenerBlock(
                        kAudioObjectSystemObject as u32,
                        NonNull::from(&a),
                        Some(&self.queue),
                        RcBlock::as_ptr(block),
                    );
                }
            }
            // Any callback already queued runs before this returns.
            self.queue.exec_sync(|| {});
        }
    }
}
