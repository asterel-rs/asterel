use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

const FIXED_SEED: u64 = 0xA57E_40F6_2A13_9D11;

pub fn for_each_fuzz_input(iterations: usize, max_len: usize, mut f: impl FnMut(&[u8])) {
    let edge_cases: [&[u8]; 12] = [
        b"",
        b"\0",
        b"..",
        b"%2e%2e",
        b"..%2f",
        b"%2f..",
        b"localhost",
        b"127.0.0.1",
        b"fc00::1",
        b"```json\n{}\n```",
        b"\xEF\xBB\xBF",
        &[0xFF, 0xFE, 0xFD],
    ];
    for case in edge_cases {
        f(case);
    }

    let mut rng = StdRng::seed_from_u64(FIXED_SEED);
    for _ in 0..iterations {
        let len = rng.random_range(0..=max_len);
        let mut data = vec![0_u8; len];
        rng.fill(&mut data[..]);
        f(&data);
    }
}
