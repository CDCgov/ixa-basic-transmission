use std::{
    any::TypeId,
    collections::HashMap,
    sync::{LazyLock, Mutex, atomic::AtomicUsize},
};

pub static TYPE_STORE_INDEXER: LazyLock<Mutex<HashMap<TypeId, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub static TYPE_STORE_SIZES: LazyLock<Mutex<HashMap<TypeId, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub trait TypeIndexCategory: 'static {
    fn category_id() -> TypeId {
        TypeId::of::<Self>()
    }
}

fn maybe_register_type_index_category<C: TypeIndexCategory>() {
    let category_type_id = C::category_id();
    let mut map = TYPE_STORE_SIZES.lock().unwrap();
    let entry = map.entry(category_type_id).or_insert_with(|| 0);
    *entry += 1;
}

pub trait TypeIndex<C: TypeIndexCategory> {
    fn type_index() -> usize;
}

pub fn maybe_init_type_index<C: TypeIndexCategory>(index_holder: &AtomicUsize) -> usize {
    // Fast path: already initialized.
    let index = index_holder.load(std::sync::atomic::Ordering::Relaxed);
    if index != usize::MAX {
        return index;
    }

    let mut map = TYPE_STORE_INDEXER.lock().unwrap();
    let category_type_id = C::category_id();
    let candidate_ref = map.entry(category_type_id).or_insert_with(|| 0);
    let candidate = *candidate_ref;

    // Try to claim the candidate index. Here we guard against the potential race condition that
    // another instance in another thread just initialized the index prior to us
    // obtaining the lock.
    match index_holder.compare_exchange(
        usize::MAX,
        candidate,
        std::sync::atomic::Ordering::AcqRel,
        std::sync::atomic::Ordering::Acquire,
    ) {
        Ok(_) => {
            // We won the race — increment the global next plugin index and return the new index
            *candidate_ref += 1;
            candidate
        }
        Err(existing) => {
            // Another thread beat us — don’t increment the global next plugin index,
            // just return existing
            existing
        }
    }
}

#[macro_export]
macro_rules! type_index {
    ($category:ty, $key:ty) => {
        paste::paste! {
            impl $crate::ixa_plus::type_index::TypeIndex<$category> for $key {
                fn type_index() -> usize {
                    // This static must be initialized with a compile-time constant expression.
                    // We use `usize::MAX` as a sentinel to mean "uninitialized". This
                    // static variable is shared among all instances of this data plugin type.
                    static INDEX: std::sync::atomic::AtomicUsize =
                        std::sync::atomic::AtomicUsize::new(usize::MAX);

                    // Return the index or initialize it.
                    $crate::ixa_plus::type_index::maybe_init_type_index::<$category>(&INDEX)
                }
            }
        }
    };
}

pub struct TypeIndexMap<C: TypeIndexCategory, T> {
    _maker: std::marker::PhantomData<C>,
    store: Vec<Option<T>>,
}

impl<C: TypeIndexCategory, V> TypeIndexMap<C, V> {
    /// Creates an empty TypeIndexMap and registers the category if needed.
    pub fn new() -> Self {
        Self::with_capacity(0)
    }

    /// Creates a TypeIndexMap with the given capacity and registers the category if needed.
    pub fn with_capacity(capacity: usize) -> Self {
        maybe_register_type_index_category::<C>();
        Self {
            _maker: std::marker::PhantomData,
            store: Vec::with_capacity(capacity),
        }
    }

    /// Returns a reference to the value for the given type index, if it exists.
    pub fn get<K: TypeIndex<C>>(&self) -> Option<&V> {
        let index = K::type_index();
        self.store.get(index).and_then(|v| v.as_ref())
    }

    /// Returns a mutable reference to the value for the given type index, if it exists.
    pub fn get_mut<K: TypeIndex<C>>(&mut self) -> Option<&mut V> {
        let index = K::type_index();
        self.store.get_mut(index).and_then(|v| v.as_mut())
    }

    pub fn get_mut_or_insert<K: TypeIndex<C>>(&mut self, value: V) -> &mut V {
        let index = K::type_index();
        if index >= self.store.len() {
            self.store.resize_with(index + 1, || None);
        }
        self.store[index].get_or_insert(value)
    }

    /// Inserts a value for the given type index.
    pub fn insert<K: TypeIndex<C>>(&mut self, value: V) {
        let index = K::type_index();
        if index >= self.store.len() {
            self.store.resize_with(index + 1, || None);
        }
        self.store[index] = Some(value);
    }

    /// Removes and returns the value for the given type index, if it exists.
    pub fn remove<K: TypeIndex<C>>(&mut self) -> Option<V> {
        let index = K::type_index();
        if index >= self.store.len() {
            return None;
        }
        self.store[index].take()
    }

    /// Returns an iterator over values in canonical type order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.store.iter().filter_map(|v| v.as_ref())
    }
}
