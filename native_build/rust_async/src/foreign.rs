//! One-shot C completion with an explicit foreign ownership/threading contract.
use std::ffi::c_void;

struct TransferredContext(*mut c_void);

// SAFETY: only ForeignCompletion::new can construct this private value. Its
// caller guarantees that the context is valid on the completion thread until
// callback return. No Rust access to its pointee is exposed. This does not make
// arbitrary pointers Send, nor does it make the foreign context Sync or owned.
unsafe impl Send for TransferredContext {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ForeignFailure {
    Failed = 3,
    Cancelled = 4,
    Saturated = 5,
}

/// One completion authority. A successful outcome always has a borrowed payload;
/// a failed outcome never carries a payload. Fields are private and completing
/// consumes the value. Rejected admission drops it without invoking the callback.
///
/// ```compile_fail
/// use native_async_runtime::foreign::ForeignCompletion;
/// fn wrong(completion: ForeignCompletion<u64>) {
///     completion.complete(Ok(1));
///     completion.complete(Ok(2)); // authority was consumed
/// }
/// ```
/// ```compile_fail
/// use native_async_runtime::foreign::ForeignCompletion;
/// fn wrong(completion: ForeignCompletion<u64>) {
///     completion.complete(Err(0)); // success cannot be an error status
/// }
/// ```
#[must_use = "complete accepted work; discard only on rejected admission"]
pub struct ForeignCompletion<T> {
    callback: unsafe extern "C" fn(*mut c_void, u32, *const T),
    context: TransferredContext,
}

impl<T> ForeignCompletion<T> {
    /// Transfer callback authority at the audited FFI boundary.
    ///
    /// # Safety
    /// `context` must remain valid until callback return (or rejected admission),
    /// including when callback execution precedes submission return. It must be
    /// usable on the receiving native thread. `callback` must accept status 0
    /// with a valid borrowed T, or ForeignFailure with a null payload, and must
    /// neither retain that payload nor unwind. T's layout must match the foreign
    /// declaration. The callback must return promptly without waiting for another
    /// workflow. No thread-affine JavaScript values may be accessed here.
    pub unsafe fn new(
        context: *mut c_void,
        callback: unsafe extern "C" fn(*mut c_void, u32, *const T),
    ) -> Self {
        Self {
            callback,
            context: TransferredContext(context),
        }
    }

    pub fn complete(self, outcome: Result<T, ForeignFailure>) {
        // SAFETY: constructor established the foreign callback contract. A local
        // success payload is alive through callback return; failures carry null.
        match outcome {
            Ok(value) => unsafe { (self.callback)(self.context.0, 0, &value) },
            Err(error) => unsafe {
                (self.callback)(self.context.0, error as u32, std::ptr::null())
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    unsafe extern "C" fn receive(context: *mut c_void, status: u32, value: *const u64) {
        let calls = unsafe { &*context.cast::<AtomicU32>() };
        if status == 0 {
            assert_eq!(unsafe { *value }, 42);
        } else {
            assert_eq!(status, ForeignFailure::Cancelled as u32);
            assert!(value.is_null());
        }
        calls.fetch_add(1, Ordering::Release);
    }

    #[test]
    fn typed_outcomes_cross_the_callback_boundary_once() {
        let mut calls = AtomicU32::new(0);
        let pointer = (&raw mut calls).cast();
        // Context stays alive, is thread-safe, and the callback consumes its data.
        unsafe { ForeignCompletion::new(pointer, receive) }.complete(Ok(42));
        unsafe { ForeignCompletion::new(pointer, receive) }
            .complete(Err(ForeignFailure::Cancelled));
        assert_eq!(calls.load(Ordering::Acquire), 2);
        {
            let _rejected = unsafe { ForeignCompletion::new(pointer, receive) };
        }
        assert_eq!(calls.load(Ordering::Acquire), 2); // rejection invokes nothing
    }
}
