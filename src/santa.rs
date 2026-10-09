use rand::{Rng, seq::SliceRandom};

/// Pairs every person with someone to give a gift to, returned as `(giver, receiver)`.
///
/// The people are shuffled and linked into a single circle, where each person gives to the
/// next one. This guarantees nobody draws themselves and avoids small closed loops
/// (A gives to B and B gives to A) that would make it easy to guess who your Santa is.
pub fn draw<T: Clone, R: Rng + ?Sized>(people: &[T], rng: &mut R) -> Vec<(T, T)> {
    let mut order = people.to_vec();
    order.shuffle(rng);
    let n = order.len();
    (0..n)
        .map(|i| (order[i].clone(), order[(i + 1) % n].clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn everyone_gives_and_receives_exactly_once_and_never_to_themselves() {
        let mut rng = rand::rng();
        for n in 2..20 {
            let people: Vec<u32> = (0..n).collect();
            for _ in 0..50 {
                let pairs = draw(&people, &mut rng);
                assert_eq!(pairs.len(), people.len());

                let givers: HashSet<_> = pairs.iter().map(|(g, _)| *g).collect();
                let receivers: HashSet<_> = pairs.iter().map(|(_, r)| *r).collect();
                assert_eq!(givers.len(), people.len());
                assert_eq!(receivers.len(), people.len());
                assert!(pairs.iter().all(|(g, r)| g != r));
            }
        }
    }

    #[test]
    fn forms_a_single_circle() {
        let mut rng = rand::rng();
        let people: Vec<u32> = (0..10).collect();
        let pairs: HashMap<_, _> = draw(&people, &mut rng).into_iter().collect();

        let mut current = people[0];
        for _ in 0..people.len() - 1 {
            current = pairs[&current];
            assert_ne!(current, people[0], "found a loop shorter than the whole group");
        }
        assert_eq!(pairs[&current], people[0]);
    }

    #[test]
    fn empty_input_gives_no_pairs() {
        assert!(draw::<u32, _>(&[], &mut rand::rng()).is_empty());
    }
}
