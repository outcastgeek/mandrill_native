//! Owned immutable cell values retained by the same journal and commit gate.

use super::journal::JournalError;
use crate::memory::{FramePool, FrameReservation, MemoryError, SharedRuntimeFrame};
use crate::transactional::TxValue;
use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::Arc;

pub(crate) struct ErasedPreparedValue {
    frame: SharedRuntimeFrame,
    ty: TypeId,
}

pub(crate) enum ErasedValue {
    Ordinary(Arc<dyn Any + Send + Sync>),
    Prepared(ErasedPreparedValue),
}

impl ErasedValue {
    pub(crate) fn ordinary<T: Any + Send + Sync>(value: Arc<T>) -> Self {
        Self::Ordinary(value)
    }

    pub(crate) fn try_clone(&self) -> Result<Self, MemoryError> {
        match self {
            Self::Ordinary(value) => Ok(Self::Ordinary(Arc::clone(value))),
            Self::Prepared(value) => Ok(Self::Prepared(ErasedPreparedValue {
                frame: value.frame.try_clone()?,
                ty: value.ty,
            })),
        }
    }

    pub(crate) fn check_type(&self, replacement: &Self) -> Result<(), JournalError> {
        let same = match (self, replacement) {
            (Self::Ordinary(first), Self::Ordinary(second)) => {
                first.as_ref().type_id() == second.as_ref().type_id()
            }
            (Self::Prepared(first), Self::Prepared(second)) => first.ty == second.ty,
            (Self::Ordinary(_), Self::Prepared(_)) | (Self::Prepared(_), Self::Ordinary(_)) => {
                false
            }
        };
        if same {
            Ok(())
        } else {
            Err(JournalError::WrongValueType)
        }
    }

    pub(crate) fn into_ordinary<T: Any + Send + Sync>(self) -> Result<Arc<T>, JournalError> {
        match self {
            Self::Ordinary(value) => Arc::downcast(value).map_err(|_| JournalError::WrongValueType),
            Self::Prepared(_) => Err(JournalError::WrongValueType),
        }
    }

    pub(crate) fn into_prepared<T: TxValue>(self) -> Result<PreparedTxValue<T>, JournalError> {
        match self {
            Self::Prepared(value) if value.ty == TypeId::of::<T>() => Ok(PreparedTxValue {
                frame: value.frame,
                marker: PhantomData,
            }),
            Self::Ordinary(_) | Self::Prepared(_) => Err(JournalError::WrongValueType),
        }
    }
}

/// An immutable transaction value retaining its actual regional payload.
/// Checked aliases keep the same generation alive across commits.
pub struct PreparedTxValue<T: TxValue> {
    frame: SharedRuntimeFrame,
    marker: PhantomData<fn() -> T>,
}

impl<T: TxValue> PreparedTxValue<T> {
    pub(in crate::transactional) fn publish(
        reservation: FrameReservation<'static, T>,
        value: T,
    ) -> Self {
        Self {
            frame: reservation.share(value),
            marker: PhantomData,
        }
    }

    pub(in crate::transactional) fn into_erased(self) -> ErasedValue {
        ErasedValue::Prepared(ErasedPreparedValue {
            frame: self.frame,
            ty: TypeId::of::<T>(),
        })
    }

    pub(in crate::transactional) fn belongs_to(&self, frames: &FramePool<'static>) -> bool {
        frames.owns_shared(&self.frame)
    }

    /// Retain the same immutable payload without copying `T`.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            frame: self.frame.try_clone()?,
            marker: PhantomData,
        })
    }
}

impl<T: TxValue> Deref for PreparedTxValue<T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: publish stores a T; erasure keeps its TypeId and reconstruction
        // checks that identity. The counted frame keeps it initialized and shared.
        unsafe { self.frame.pointer().cast::<T>().as_ref() }
    }
}
