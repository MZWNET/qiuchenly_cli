use serde::Deserialize;

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum OneOrMany<T> {
    Many(Vec<T>),
    One(T),
}

impl<T> OneOrMany<T> {
    pub(super) fn as_slice(&self) -> &[T] {
        match self {
            OneOrMany::One(item) => std::slice::from_ref(item),
            OneOrMany::Many(items) => items,
        }
    }

    pub(super) fn iter(&self) -> std::slice::Iter<'_, T> {
        self.as_slice().iter()
    }

    fn into_vec(self) -> Vec<T> {
        match self {
            OneOrMany::One(item) => vec![item],
            OneOrMany::Many(items) => items,
        }
    }

    pub(super) fn append(self, other: OneOrMany<T>) -> Self {
        let mut items = self.into_vec();
        match other {
            OneOrMany::One(item) => items.push(item),
            OneOrMany::Many(other) => items.extend(other),
        }
        OneOrMany::Many(items)
    }
}

impl<T> Default for OneOrMany<T> {
    fn default() -> Self {
        OneOrMany::Many(Vec::new())
    }
}
