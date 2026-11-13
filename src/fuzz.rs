//! Randomized tests: inputs built from fragments that matter to a parser, many times over.
//! `PARCELA_FUZZ_ROUNDS` sets how many (the nightly stress run raises it) and `PARCELA_FUZZ_SEED`
//! replays the seed a failure printed.

pub struct Rng(u64);
impl Rng {
    pub fn below(&mut self, bound: usize) -> usize {
        // xorshift64*: deterministic for a seed, which is all a replay needs.
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 32) as usize % bound.max(1)
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// Up to `most` fragments joined, with an occasional random byte between them.
    pub fn text(&mut self, fragments: &[&[u8]], most: usize) -> Vec<u8> {
        let mut text = Vec::new();
        for _ in 0..self.below(most + 1) {
            if self.below(8) == 0 {
                text.push(self.below(256) as u8);
            } else {
                let fragment: &[u8] = fragments[self.below(fragments.len())];
                text.extend_from_slice(fragment);
            }
        }
        text
    }
}

/// Runs `case` once per round with its own generator, and names the seed of a round that fails.
pub fn rounds(default: usize, mut case: impl FnMut(&mut Rng)) {
    let var = |name| {
        std::env::var(name)
            .ok()
            .and_then(|value: String| value.parse().ok())
    };
    let (first, count) = match var("PARCELA_FUZZ_SEED") {
        Some(seed) => (seed, 1),
        None => (
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |time| time.as_nanos() as u64),
            var("PARCELA_FUZZ_ROUNDS").map_or(default, |rounds: u64| rounds as usize),
        ),
    };
    for round in 0..count as u64 {
        let seed = first.wrapping_add(round.wrapping_mul(0x9e37_79b9_7f4a_7c15)) | 1;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            case(&mut Rng(seed));
        }));
        if let Err(panic) = result {
            eprintln!("failed with PARCELA_FUZZ_SEED={seed}");
            std::panic::resume_unwind(panic);
        }
    }
}
