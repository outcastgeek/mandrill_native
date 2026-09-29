use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::sync::Arc;

/// Opt-in immutable transactional value. Implementations promise that no
/// aliases can mutate observable contents outside a transaction. Built-in
/// implementations are limited to recursively immutable shapes; a custom
/// type with interior mutability must not implement this contract.
pub trait TxValue: Any + Send + Sync {}

macro_rules! immutable_scalar {
    ($($name:ty),* $(,)?) => { $(impl TxValue for $name {})* };
}

immutable_scalar!(
    (),
    bool,
    char,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    f32,
    f64,
    String,
);

impl TxValue for Arc<str> {}
impl TxValue for Arc<[u8]> {}
impl<T: TxValue> TxValue for Option<T> {}
impl<T: TxValue> TxValue for Arc<T> {}
impl<T: TxValue> TxValue for Vec<T> {}
impl<T: TxValue> TxValue for VecDeque<T> {}
impl<T: TxValue + Ord> TxValue for BTreeSet<T> {}
impl<K: TxValue + Ord, V: TxValue> TxValue for BTreeMap<K, V> {}
impl<T: TxValue + Eq + Hash> TxValue for HashSet<T> {}
impl<K: TxValue + Eq + Hash, V: TxValue> TxValue for HashMap<K, V> {}
impl<A: TxValue, B: TxValue> TxValue for (A, B) {}
impl<A: TxValue, B: TxValue, C: TxValue> TxValue for (A, B, C) {}
