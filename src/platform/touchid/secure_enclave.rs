//! Ownership boundary for the CryptoKit bridge; returned CF objects are retained.

use core_foundation::{
    base::TCFType,
    data::{
        CFData, CFDataAppendBytes, CFDataCreateMutable, CFDataGetMutableBytePtr, CFDataRef,
        CFMutableDataRef,
    },
    error::{CFError, CFErrorRef},
};
use objc2_local_authentication::LAContext;
use std::{ffi::c_void, ptr};
use zeroize::{Zeroize, Zeroizing};

unsafe extern "C" {
    fn lockbox_touch_id_enclave_available() -> bool;
    fn lockbox_touch_id_seal(
        input: CFDataRef,
        account: CFDataRef,
        error: *mut CFErrorRef,
    ) -> CFDataRef;
    fn lockbox_touch_id_open(
        input: CFDataRef,
        account: CFDataRef,
        context: *const c_void,
        error: *mut CFErrorRef,
    ) -> CFDataRef;
}

pub(super) fn available() -> bool {
    unsafe { lockbox_touch_id_enclave_available() }
}

pub(super) fn seal(data: &[u8], account: &str) -> Result<Vec<u8>, CFError> {
    let data = SecretData::new(data);
    let account = CFData::from_buffer(account.as_bytes());
    let mut error = ptr::null_mut();
    let output = unsafe {
        lockbox_touch_id_seal(
            data.0.as_concrete_TypeRef(),
            account.as_concrete_TypeRef(),
            &mut error,
        )
    };
    // Swift returns exactly one retained result or error, never both.
    if output.is_null() {
        return Err(unsafe { CFError::wrap_under_create_rule(error) });
    }
    Ok(unsafe { CFData::wrap_under_create_rule(output) }.to_vec())
}

pub(super) fn open(
    data: &[u8],
    account: &str,
    context: &LAContext,
) -> Result<Zeroizing<Vec<u8>>, CFError> {
    let data = CFData::from_buffer(data);
    let account = CFData::from_buffer(account.as_bytes());
    let mut error = ptr::null_mut();
    let output = unsafe {
        lockbox_touch_id_open(
            data.as_concrete_TypeRef(),
            account.as_concrete_TypeRef(),
            (context as *const LAContext).cast(),
            &mut error,
        )
    };
    if output.is_null() {
        return Err(unsafe { CFError::wrap_under_create_rule(error) });
    }
    // The bridge guarantees an exclusively owned CFMutableData result.
    let output = SecretData(unsafe { CFData::wrap_under_create_rule(output) });
    Ok(Zeroizing::new(output.0.to_vec()))
}

struct SecretData(CFData);

impl SecretData {
    fn new(bytes: &[u8]) -> Self {
        unsafe {
            let data = CFDataCreateMutable(ptr::null(), bytes.len() as isize);
            CFDataAppendBytes(data, bytes.as_ptr(), bytes.len() as isize);
            Self(CFData::wrap_under_create_rule(data))
        }
    }
}

impl Drop for SecretData {
    fn drop(&mut self) {
        if !self.0.is_empty() {
            unsafe {
                std::slice::from_raw_parts_mut(
                    CFDataGetMutableBytePtr(self.0.as_concrete_TypeRef() as CFMutableDataRef),
                    self.0.len() as usize,
                )
            }
            .zeroize();
        }
    }
}
