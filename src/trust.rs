use std::ffi::CStr;

use ffi;
use Wrapper;

pub struct TrustItem {
    raw: ffi::gpgme_trust_item_t,
}

impl Drop for TrustItem {
    fn drop(&mut self) {
        unsafe { ffi::gpgme_trust_item_unref(self.raw) }
    }
}

impl Clone for TrustItem {
    fn clone(&self) -> TrustItem {
        unsafe {
            ffi::gpgme_trust_item_ref(self.raw);
            TrustItem { raw: self.raw }
        }
    }
}

unsafe impl Wrapper for TrustItem {
    type Raw = ffi::gpgme_trust_item_t;

    unsafe fn from_raw(raw: ffi::gpgme_trust_item_t) -> TrustItem {
        debug_assert!(!raw.is_null());
        TrustItem { raw: raw }
    }

    fn as_raw(&self) -> ffi::gpgme_trust_item_t {
        self.raw
    }
}

impl TrustItem {
    pub fn level(&self) -> i32 {
        unsafe { (*self.raw).level.into() }
    }

    pub fn key_id_raw(&self) -> Option<&CStr> {
        unsafe { (*self.raw).keyid.as_ref().map(|s| CStr::from_ptr(s)) }
    }

    pub fn owner_trust_raw(&self) -> Option<&CStr> {
        unsafe { (*self.raw).owner_trust.as_ref().map(|s| CStr::from_ptr(s)) }
    }

    pub fn name_raw(&self) -> Option<&CStr> {
        unsafe { (*self.raw).name.as_ref().map(|s| CStr::from_ptr(s)) }
    }

    pub fn validity_raw(&self) -> Option<&CStr> {
        unsafe { (*self.raw).validity.as_ref().map(|s| CStr::from_ptr(s)) }
    }
}
