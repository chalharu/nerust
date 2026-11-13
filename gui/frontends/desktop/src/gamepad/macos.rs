//! macOS HID reader: IOHIDManager poll into SDL-indexed snapshots.
//!
//! Synchronous `IOHIDDeviceGetValue` reads each frame keep this layer
//! callback- and thread-free; the UI frame loop already ticks at 60 Hz,
//! so no reader thread or runloop source is needed. Elements enumerate in
//! SDL order (buttons by HID usage, axes X/Y/Z/Rx/Ry/Rz/Slider/Dial/Wheel,
//! hats as listed), matching the SDL hidapi numbering the DB tables assume
//! for standard USB pads. Bluetooth pairs needing report handshakes stay a
//! Phase 3 follow-up; they surface here as unknown pads, never as silence.

use core_foundation_sys::{
    base::{CFIndex, CFRelease, CFTypeRef, kCFAllocatorDefault},
    set::{CFSetGetCount, CFSetGetValues, CFSetRef},
    string::{CFStringCreateWithCString, CFStringGetCString, CFStringRef, kCFStringEncodingUTF8},
};
use io_kit_sys::{
    hid::{
        base::{IOHIDDeviceRef, IOHIDElementRef, IOHIDValueRef},
        device::{
            IOHIDDeviceClose, IOHIDDeviceConformsTo, IOHIDDeviceCopyMatchingElements,
            IOHIDDeviceGetProperty, IOHIDDeviceGetValue, IOHIDDeviceOpen,
        },
        element::{
            IOHIDElementGetLogicalMax, IOHIDElementGetLogicalMin, IOHIDElementGetUsage,
            IOHIDElementGetUsagePage,
        },
        keys::kIOHIDOptionsTypeNone,
        manager::{
            IOHIDManagerClose, IOHIDManagerCopyDevices, IOHIDManagerCreate, IOHIDManagerOpen,
            IOHIDManagerRef, kIOHIDManagerOptionNone,
        },
        value::IOHIDValueGetIntegerValue,
    },
    ret::kIOReturnSuccess,
};
use std::ffi::CString;
use std::ptr::{null, null_mut};

use super::Snapshot;

/// HID usage pages and usages this reader classifies.
const PAGE_GENERIC_DESKTOP: u32 = 0x01;
const PAGE_BUTTON: u32 = 0x09;
/// Axis usages in SDL `aN` order: X, Y, Z, Rx, Ry, Rz, Slider, Dial, Wheel.
const AXIS_USAGES: [u32; 9] = [0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38];
const USAGE_HAT: u32 = 0x39;
const USAGE_JOYSTICK: u32 = 0x04;
const USAGE_GAMEPAD: u32 = 0x05;
const USAGE_MULTI_AXIS: u32 = 0x08;
/// HID hat positions to SDL masks (`HAT_UP/RIGHT/DOWN/LEFT` = 1/2/4/8).
const HAT_MASKS: [u8; 8] = [1, 3, 2, 6, 4, 12, 8, 9];

/// One open device: retained by the manager, classified once at connect.
struct OpenDevice {
    device: IOHIDDeviceRef,
    key: String,
    vid: u16,
    pid: u16,
    buttons: Vec<IOHIDElementRef>,
    axes: Vec<AxisElement>,
    hats: Vec<IOHIDElementRef>,
}

struct AxisElement {
    element: IOHIDElementRef,
    min: i64,
    max: i64,
}

/// Persistent IOHID session: manager plus its open devices. UI-thread
/// owned; never shared, so no `Send` is implemented.
pub struct Backend {
    manager: IOHIDManagerRef,
    devices: Vec<OpenDevice>,
}

impl Backend {
    /// Manager open failure warns once and parks as empty; every later
    /// poll stays empty instead of retrying noisily.
    pub fn new() -> Self {
        let manager = unsafe { IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDManagerOptionNone) };
        if manager.is_null() {
            log::warn!("gamepad: IOHIDManagerCreate failed, pads disabled");
            return Self {
                manager: null_mut(),
                devices: Vec::new(),
            };
        }
        if unsafe { IOHIDManagerOpen(manager, kIOHIDManagerOptionNone) } != kIOReturnSuccess {
            log::warn!("gamepad: IOHIDManagerOpen failed, pads disabled");
            unsafe { CFRelease(manager as CFTypeRef) };
            return Self {
                manager: null_mut(),
                devices: Vec::new(),
            };
        }
        Self {
            manager,
            devices: Vec::new(),
        }
    }

    /// One synchronous poll: reconcile the device set, then read every
    /// open element into SDL-indexed snapshots.
    pub fn poll(&mut self) -> Vec<Snapshot> {
        if self.manager.is_null() {
            return Vec::new();
        }
        let present = copy_devices(self.manager);
        let mut seen = Vec::with_capacity(present.len());
        for device in present {
            if unsafe {
                IOHIDDeviceConformsTo(device, PAGE_GENERIC_DESKTOP, USAGE_JOYSTICK) == 0
                    && IOHIDDeviceConformsTo(device, PAGE_GENERIC_DESKTOP, USAGE_GAMEPAD) == 0
                    && IOHIDDeviceConformsTo(device, PAGE_GENERIC_DESKTOP, USAGE_MULTI_AXIS) == 0
            } {
                continue;
            }
            let (vid, pid, location, serial, product) = device_identity(device);
            let key = if serial.is_empty() {
                format!("HID:{vid:04X}:{pid:04X}:{location:08X}")
            } else {
                format!("HID:{vid:04X}:{pid:04X}:{location:08X}:{serial}")
            };
            seen.push(key.clone());
            if self.devices.iter().any(|open| open.key == key) {
                continue;
            }
            match open_device(device, &key, vid, pid) {
                Some(opened) => {
                    log::info!("gamepad: connected {product} ({key})");
                    self.devices.push(opened);
                }
                None => log::warn!("gamepad: skipping {product} ({key})"),
            }
        }
        let mut gone = Vec::new();
        self.devices.retain(|open| {
            if seen.contains(&open.key) {
                true
            } else {
                gone.push((open.device, open.key.clone()));
                false
            }
        });
        for (device, key) in gone {
            unsafe {
                IOHIDDeviceClose(device, kIOHIDOptionsTypeNone);
            }
            log::info!("gamepad: disconnected {key}");
        }
        self.devices
            .iter()
            .filter(|open| seen.contains(&open.key))
            .map(read_snapshot)
            .collect()
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        for open in &self.devices {
            unsafe {
                IOHIDDeviceClose(open.device, kIOHIDOptionsTypeNone);
            }
        }
        if !self.manager.is_null() {
            unsafe {
                IOHIDManagerClose(self.manager, kIOHIDManagerOptionNone);
                CFRelease(self.manager as CFTypeRef);
            }
        }
    }
}

/// Current device set: borrowed refs, valid until the manager changes.
fn copy_devices(manager: IOHIDManagerRef) -> Vec<IOHIDDeviceRef> {
    unsafe {
        let set: CFSetRef = IOHIDManagerCopyDevices(manager);
        if set.is_null() {
            return Vec::new();
        }
        let count = CFSetGetCount(set) as usize;
        let mut values = vec![null::<std::ffi::c_void>(); count];
        CFSetGetValues(set, values.as_mut_ptr());
        CFRelease(set as CFTypeRef);
        values
            .into_iter()
            .map(|value| value as IOHIDDeviceRef)
            .collect()
    }
}

/// `(vid, pid, location, serial, product)`: property reads need no open
/// device, so identity computes before the exclusive open attempt.
fn device_identity(device: IOHIDDeviceRef) -> (u16, u16, u32, String, String) {
    let vid = get_number(device, "VendorID") as u16;
    let pid = get_number(device, "ProductID") as u16;
    let location = get_number(device, "LocationID") as u32;
    let serial = get_string(device, "SerialNumber");
    let mut product = get_string(device, "Product");
    if product.is_empty() {
        product = format!("{vid:04X}:{pid:04X}");
    }
    (vid, pid, location, serial, product)
}

/// Open plus classify: probe-read every candidate so collections and
/// unreadable elements never enter the tables.
fn open_device(device: IOHIDDeviceRef, key: &str, vid: u16, pid: u16) -> Option<OpenDevice> {
    if unsafe { IOHIDDeviceOpen(device, kIOHIDOptionsTypeNone) } != kIOReturnSuccess {
        return None;
    }
    let mut buttons: Vec<(u32, IOHIDElementRef)> = Vec::new();
    let mut axes: Vec<(u32, IOHIDElementRef, i64, i64)> = Vec::new();
    let mut hats: Vec<IOHIDElementRef> = Vec::new();
    unsafe {
        let elements = IOHIDDeviceCopyMatchingElements(device, null(), kIOHIDOptionsTypeNone);
        if elements.is_null() {
            IOHIDDeviceClose(device, kIOHIDOptionsTypeNone);
            return None;
        }
        let count = core_foundation_sys::array::CFArrayGetCount(elements) as usize;
        for index in 0..count {
            let element =
                core_foundation_sys::array::CFArrayGetValueAtIndex(elements, index as CFIndex)
                    as IOHIDElementRef;
            if element.is_null() || read_value(device, element).is_none() {
                continue;
            }
            let page = IOHIDElementGetUsagePage(element);
            let usage = IOHIDElementGetUsage(element);
            if page == PAGE_BUTTON {
                buttons.push((usage, element));
            } else if page == PAGE_GENERIC_DESKTOP {
                if AXIS_USAGES.contains(&usage) {
                    let min = IOHIDElementGetLogicalMin(element) as i64;
                    let max = IOHIDElementGetLogicalMax(element) as i64;
                    if max > min {
                        axes.push((usage, element, min, max));
                    }
                } else if usage == USAGE_HAT {
                    hats.push(element);
                }
            }
        }
        CFRelease(elements as CFTypeRef);
    }
    buttons.sort_by_key(|(usage, _)| *usage);
    axes.sort_by_key(|(usage, _, _, _)| *usage);
    Some(OpenDevice {
        device,
        key: key.to_owned(),
        vid,
        pid,
        buttons: buttons.into_iter().map(|(_, element)| element).collect(),
        axes: axes
            .into_iter()
            .map(|(_, element, min, max)| AxisElement { element, min, max })
            .collect(),
        hats,
    })
}

/// Full frame read: buttons in usage order, axes normalized `-1..=1`,
/// hats as SDL masks. A failed mid-poll read fails safe to neutral.
fn read_snapshot(open: &OpenDevice) -> Snapshot {
    let buttons = open
        .buttons
        .iter()
        .map(|element| read_value(open.device, *element).is_some_and(|value| value != 0))
        .collect();
    let axes = open
        .axes
        .iter()
        .map(|axis| {
            let value = read_value(open.device, axis.element).unwrap_or(0) as f64;
            let span = (axis.max - axis.min) as f64;
            (((value - axis.min as f64) / span) * 2.0 - 1.0).clamp(-1.0, 1.0) as f32
        })
        .collect();
    let hats = open
        .hats
        .iter()
        .map(|element| {
            let value = read_value(open.device, *element).unwrap_or(-1);
            if !(0..=7).contains(&value) {
                0
            } else {
                HAT_MASKS[value as usize]
            }
        })
        .collect();
    Snapshot {
        descriptor: open.key.clone(),
        vid: open.vid,
        pid: open.pid,
        buttons,
        axes,
        hats,
    }
}

/// Single synchronous element read; `None` on any I/O failure. The value
/// is caller-owned and released here.
fn read_value(device: IOHIDDeviceRef, element: IOHIDElementRef) -> Option<i64> {
    unsafe {
        let mut value: IOHIDValueRef = null_mut();
        if IOHIDDeviceGetValue(device, element, &mut value) != kIOReturnSuccess || value.is_null() {
            return None;
        }
        let integer = IOHIDValueGetIntegerValue(value) as i64;
        CFRelease(value as CFTypeRef);
        Some(integer)
    }
}

/// Numeric device property, `0` when absent or unreadable.
fn get_number(device: IOHIDDeviceRef, name: &str) -> i64 {
    unsafe {
        let key = cf_string(name);
        if key.is_null() {
            return 0;
        }
        let number =
            IOHIDDeviceGetProperty(device, key) as core_foundation_sys::number::CFNumberRef;
        CFRelease(key as CFTypeRef);
        if number.is_null() {
            return 0;
        }
        let mut value: i64 = 0;
        if !core_foundation_sys::number::CFNumberGetValue(
            number,
            core_foundation_sys::number::kCFNumberSInt64Type,
            &mut value as *mut i64 as *mut std::ffi::c_void,
        ) {
            return 0;
        }
        value
    }
}

/// String device property, empty when absent or unreadable.
fn get_string(device: IOHIDDeviceRef, name: &str) -> String {
    unsafe {
        let key = cf_string(name);
        if key.is_null() {
            return String::new();
        }
        let string = IOHIDDeviceGetProperty(device, key) as CFStringRef;
        CFRelease(key as CFTypeRef);
        if string.is_null() {
            return String::new();
        }
        let mut buffer = [0 as std::os::raw::c_char; 256];
        if CFStringGetCString(
            string,
            buffer.as_mut_ptr(),
            buffer.len() as CFIndex,
            kCFStringEncodingUTF8,
        ) == 0
        {
            return String::new();
        }
        std::ffi::CStr::from_ptr(buffer.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

/// Autoreleased-free key string: created per property read (connect-time
/// only, never on the poll hot path) and released by the caller.
fn cf_string(name: &str) -> CFStringRef {
    let Ok(cstring) = CString::new(name) else {
        return null();
    };
    unsafe {
        CFStringCreateWithCString(kCFAllocatorDefault, cstring.as_ptr(), kCFStringEncodingUTF8)
    }
}
