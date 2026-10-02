//! [`sin`] and [`cos`] for the glyphs' angles (and the desktop's other circles), bit for bit
//! what `f32::sin` and `f32::cos` give on wasm32: musl's `sinf` and `cosf`, which core links
//! there, without their reduction for arguments past 2^28 π/2 (about 4e8), whose code and
//! table of 2/π cost the boot about 2 KB gzipped. Past that, and for infinities and NaN, they
//! return NaN.

use core::f64::consts::{FRAC_2_PI, FRAC_PI_2};

/// sin(x) on [-π/4, π/4] to 2^-37.5, rounded to f32 (musl's `__sindf`).
fn k_sin(x: f64) -> f32 {
    const S: [f64; 4] = [
        -0.166_666_666_416_265_24,
        0.008_333_329_385_889_463,
        -1.983_933_483_609_663_2e-4,
        2.718_311_493_989_822e-6,
    ];
    let z = x * x;
    let s = z * x;
    ((x + s * (S[0] + z * S[1])) + s * (z * z) * (S[2] + z * S[3])) as f32
}

/// cos(x) on [-π/4, π/4] to 2^-34.1, rounded to f32 (musl's `__cosdf`).
fn k_cos(x: f64) -> f32 {
    const C: [f64; 4] = [
        -0.499_999_997_251_031,
        0.041_666_623_323_739_06,
        -0.001_388_676_377_460_993,
        2.439_044_879_627_741e-5,
    ];
    let z = x * x;
    let w = z * z;
    (((1.0 + z * C[0]) + w * C[1]) + (w * z) * (C[2] + z * C[3])) as f32
}

/// `x` as `n` quarter turns and a remainder `y` in about [-π/4, π/4]: below 9π/4 by musl's
/// thresholds and its multiples of π/2 rounded to f64, beyond by `__rem_pio2f`'s medium
/// path (π/2 in 25 + 53 bits); NaN past it.
fn reduce(x: f32) -> (i32, f64) {
    const TOP: [u32; 5] = [0x3f49_0fda, 0x4016_cbe3, 0x407b_53d1, 0x40af_eddf, 0x40e2_31d5];
    const TOINT: f64 = 1.5 / f64::EPSILON;
    const PIO2_1: f64 = 1.570_796_310_901_641_8;
    const PIO2_1T: f64 = 1.589_325_477_352_819_6e-8;
    let (a, x64) = (x.to_bits() & 0x7fff_ffff, f64::from(x));
    if let Some(n) = TOP.iter().position(|&t| a <= t) {
        let n = if x < 0.0 { -(n as i32) } else { n as i32 };
        return (n, x64 - f64::from(n) * FRAC_PI_2);
    }
    if a >= 0x4dc9_0fdb {
        return (0, f64::NAN);
    }
    let n = (x64 * FRAC_2_PI + TOINT) - TOINT;
    (n as i32, x64 - n * PIO2_1 - n * PIO2_1T)
}

/// The sine of the angle `n` quarter turns and `y` radians.
fn quarter(n: i32, y: f64) -> f32 {
    match n & 3 {
        0 => k_sin(y),
        1 => k_cos(y),
        2 => k_sin(-y),
        _ => -k_cos(y),
    }
}

/// The sine of `x` radians.
pub fn sin(x: f32) -> f32 {
    if x.to_bits() & 0x7fff_ffff < 0x3980_0000 {
        return x; // |x| < 2^-12
    }
    let (n, y) = reduce(x);
    quarter(n, y)
}

/// The cosine of `x` radians.
pub fn cos(x: f32) -> f32 {
    if x.to_bits() & 0x7fff_ffff < 0x3980_0000 {
        return 1.0;
    }
    let (n, y) = reduce(x);
    quarter(n + 1, y)
}
