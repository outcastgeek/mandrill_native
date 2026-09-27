#![doc = "Reusable C-ABI ownership and slice contracts for native artifacts."]

use std::slice;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BorrowedSlice<T> {
    pub data: *const T,
    pub length: usize,
}

impl<T> BorrowedSlice<T> {
    #[must_use]
    pub fn from_slice(value: &[T]) -> Self {
        Self {
            data: if value.is_empty() {
                std::ptr::null()
            } else {
                value.as_ptr()
            },
            length: value.len(),
        }
    }

    /// Borrows the referenced values without copying or taking ownership.
    ///
    /// # Safety
    /// For a non-zero length, `data` must point to `length` initialized values
    /// which remain immutable for the returned lifetime.
    pub unsafe fn as_slice<'a>(self) -> Result<&'a [T], &'static str> {
        if self.length == 0 {
            return Ok(&[]);
        }
        if self.data.is_null() {
            return Err("null C view with non-zero length");
        }
        Ok(unsafe { slice::from_raw_parts(self.data, self.length) })
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MutableSlice<T> {
    pub data: *mut T,
    pub length: usize,
    pub capacity: usize,
}

impl<T> MutableSlice<T> {
    /// Borrows the initialized portion of a caller-owned output buffer.
    ///
    /// # Safety
    /// `data` must identify `capacity` writable, properly aligned values and
    /// the caller must grant exclusive access for the returned lifetime.
    pub unsafe fn as_mut_slice<'a>(self) -> Result<&'a mut [T], &'static str> {
        if self.length > self.capacity {
            return Err("length exceeds capacity");
        }
        if self.capacity == 0 {
            return Ok(&mut []);
        }
        if self.data.is_null() {
            return Err("null C buffer with non-zero capacity");
        }
        Ok(unsafe { slice::from_raw_parts_mut(self.data, self.length) })
    }
}

pub fn catch_ffi<T>(body: impl FnOnce() -> Result<T, &'static str>) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(body))
        .ok()
        .and_then(Result::ok)
}

#[cfg(test)]
mod tests {
    use super::{BorrowedSlice, MutableSlice, catch_ffi};

    #[test]
    fn arbitrary_struct_views_are_borrowed_and_mutated() {
        #[repr(C)]
        #[derive(Clone, Copy, Debug, PartialEq)]
        struct Item {
            id: u32,
            weight: f64,
        }
        let mut values = [
            Item { id: 1, weight: 2.0 },
            Item { id: 2, weight: 3.0 },
        ];
        let borrowed = BorrowedSlice::from_slice(&values);
        assert_eq!(unsafe { borrowed.as_slice() }.ok(), Some(values.as_slice()));
        let output = MutableSlice {
            data: values.as_mut_ptr(),
            length: values.len(),
            capacity: values.len(),
        };
        let view = unsafe { output.as_mut_slice() };
        assert!(view.is_ok());
        if let Ok(items) = view
            && let Some(first) = items.first_mut()
        {
            first.id = 7;
        }
        assert_eq!(values[0].id, 7);
    }

    #[test]
    fn ffi_boundary_contains_errors() {
        assert_eq!(catch_ffi(|| Ok::<_, &'static str>(3)), Some(3));
        assert_eq!(catch_ffi(|| Err::<u32, _>("error")), None);
    }
}
