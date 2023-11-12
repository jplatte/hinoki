use itertools::Itertools as _;

#[derive(Debug)]
pub(crate) struct OrderBiMap {
    pub ordered_to_original: Vec<usize>,
    pub original_to_ordered: Vec<usize>,
}

impl OrderBiMap {
    pub(crate) fn new<T, K: Ord>(original: &[T], key_fn: impl Fn(&T) -> K) -> Self {
        let ordered_to_original: Vec<_> = original
            .iter()
            .enumerate()
            .sorted_by_key(|(_, item)| key_fn(item))
            .map(|(idx, _)| idx)
            .collect();

        let mut original_to_ordered = vec![0; original.len()];
        for (ordered, &original) in ordered_to_original.iter().enumerate() {
            original_to_ordered[original] = ordered;
        }

        Self { ordered_to_original, original_to_ordered }
    }
}
