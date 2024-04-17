#[cfg(feature = "asm")]
use crate::bn256::assembly::field_arithmetic_asm;
#[cfg(not(feature = "asm"))]
use crate::{arithmetic::macx, field_arithmetic, field_specific};

#[cfg(feature = "bn256-table")]
#[rustfmt::skip]
mod table;
#[cfg(feature = "bn256-table")]
#[cfg(test)]
mod table_tests;

#[cfg(feature = "bn256-table")]
// This table should have being generated by `build.rs`;
// and stored in `src/bn256/fr/table.rs`.
pub use table::FR_TABLE;

#[cfg(not(feature = "bn256-table"))]
use crate::impl_from_u64;

use crate::arithmetic::{adc, bigint_geq, mac, sbb};
use crate::extend_field_legendre;
use crate::ff::{FromUniformBytes, PrimeField, WithSmallOrderMulGroup};
use crate::{
    field_bits, field_common, impl_add_binop_specify_output, impl_binops_additive,
    impl_binops_additive_specify_output, impl_binops_multiplicative,
    impl_binops_multiplicative_mixed, impl_sub_binop_specify_output, impl_sum_prod,
};
use core::convert::TryInto;
use core::fmt;
use core::ops::{Add, Mul, Neg, Sub};
use rand::RngCore;
use subtle::{Choice, ConditionallySelectable, ConstantTimeEq, CtOption};

/// This represents an element of $\mathbb{F}_r$ where
///
/// `r = 0x30644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000001`
///
/// is the scalar field of the BN254 curve.
// The internal representation of this type is four 64-bit unsigned
// integers in little-endian order. `Fr` values are always in
// Montgomery form; i.e., Fr(a) = aR mod r, with R = 2^256.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fr(pub(crate) [u64; 4]);

#[cfg(feature = "derive_serde")]
crate::serialize_deserialize_32_byte_primefield!(Fr);

/// Constant representing the modulus
/// r = 0x30644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000001
const MODULUS: Fr = Fr([
    0x43e1f593f0000001,
    0x2833e84879b97091,
    0xb85045b68181585d,
    0x30644e72e131a029,
]);

/// The modulus as u32 limbs.
#[cfg(any(not(target_pointer_width = "64"), feature = "force-u32"))]
const MODULUS_LIMBS_32: [u32; 8] = [
    0xf000_0001,
    0x43e1_f593,
    0x79b9_7091,
    0x2833_e848,
    0x8181_585d,
    0xb850_45b6,
    0xe131_a029,
    0x3064_4e72,
];

const MODULUS_STR: &str = "0x30644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000001";

/// INV = -(r^{-1} mod 2^64) mod 2^64
const INV: u64 = 0xc2e1f593efffffff;

/// `R = 2^256 mod r`
/// `0xe0a77c19a07df2f666ea36f7879462e36fc76959f60cd29ac96341c4ffffffb`
const R: Fr = Fr([
    0xac96341c4ffffffb,
    0x36fc76959f60cd29,
    0x666ea36f7879462e,
    0x0e0a77c19a07df2f,
]);

/// `R^2 = 2^512 mod r`
/// `0x216d0b17f4e44a58c49833d53bb808553fe3ab1e35c59e31bb8e645ae216da7`
const R2: Fr = Fr([
    0x1bb8e645ae216da7,
    0x53fe3ab1e35c59e3,
    0x8c49833d53bb8085,
    0x0216d0b17f4e44a5,
]);

/// `R^3 = 2^768 mod r`
/// `0xcf8594b7fcc657c893cc664a19fcfed2a489cbe1cfbb6b85e94d8e1b4bf0040`
const R3: Fr = Fr([
    0x5e94d8e1b4bf0040,
    0x2a489cbe1cfbb6b8,
    0x893cc664a19fcfed,
    0x0cf8594b7fcc657c,
]);

/// `GENERATOR = 7 mod r` is a generator of the `r - 1` order multiplicative
/// subgroup, or in other words a primitive root of the field.
const GENERATOR: Fr = Fr::from_raw([0x07, 0x00, 0x00, 0x00]);

const S: u32 = 28;

/// GENERATOR^t where t * 2^s + 1 = r
/// with t odd. In other words, this
/// is a 2^s root of unity.
/// `0x3ddb9f5166d18b798865ea93dd31f743215cf6dd39329c8d34f1ed960c37c9c`
const ROOT_OF_UNITY: Fr = Fr::from_raw([
    0xd34f1ed960c37c9c,
    0x3215cf6dd39329c8,
    0x98865ea93dd31f74,
    0x03ddb9f5166d18b7,
]);

/// 1 / 2 mod r
const TWO_INV: Fr = Fr::from_raw([
    0xa1f0fac9f8000001,
    0x9419f4243cdcb848,
    0xdc2822db40c0ac2e,
    0x183227397098d014,
]);

/// 1 / ROOT_OF_UNITY mod r
const ROOT_OF_UNITY_INV: Fr = Fr::from_raw([
    0x0ed3e50a414e6dba,
    0xb22625f59115aba7,
    0x1bbe587180f34361,
    0x048127174daabc26,
]);

/// GENERATOR^{2^s} where t * 2^s + 1 = r with t odd. In other words, this is a t root of unity.
/// 0x09226b6e22c6f0ca64ec26aad4c86e715b5f898e5e963f25870e56bbe533e9a2
const DELTA: Fr = Fr::from_raw([
    0x870e56bbe533e9a2,
    0x5b5f898e5e963f25,
    0x64ec26aad4c86e71,
    0x09226b6e22c6f0ca,
]);

/// `ZETA^3 = 1 mod r` where `ZETA^2 != 1 mod r`
const ZETA: Fr = Fr::from_raw([
    0xb8ca0b2d36636f23,
    0xcc37a73fec2bc5e9,
    0x048b6e193fd84104,
    0x30644e72e131a029,
]);

impl_binops_additive!(Fr, Fr);
impl_binops_multiplicative!(Fr, Fr);
field_common!(
    Fr,
    MODULUS,
    INV,
    MODULUS_STR,
    TWO_INV,
    ROOT_OF_UNITY_INV,
    DELTA,
    ZETA,
    R,
    R2,
    R3
);
impl_sum_prod!(Fr);
extend_field_legendre!(Fr);

#[cfg(not(feature = "bn256-table"))]
impl_from_u64!(Fr, R2);
#[cfg(feature = "bn256-table")]
// A field element is represented in the montgomery form -- this allows for cheap mul_mod operations.
// The catch is, if we build an Fr element, regardless of its format, we need to perform one big integer multiplication:
//
//      Fr([val, 0, 0, 0]) * R2
//
// When the "bn256-table" feature is enabled, we read the Fr element directly from the table.
// This avoids a big integer multiplication.
//
// We use a table with 2^16 entries when the element is smaller than 2^16.
impl From<u64> for Fr {
    fn from(val: u64) -> Fr {
        if val < 65536 {
            FR_TABLE[val as usize]
        } else {
            Fr([val, 0, 0, 0]) * R2
        }
    }
}

#[cfg(not(feature = "asm"))]
field_arithmetic!(Fr, MODULUS, INV, sparse);
#[cfg(feature = "asm")]
field_arithmetic_asm!(Fr, MODULUS, INV);

#[cfg(all(target_pointer_width = "64", not(feature = "force-u32")))]
field_bits!(Fr, MODULUS);
#[cfg(any(not(target_pointer_width = "64"), feature = "force-u32"))]
field_bits!(Fr, MODULUS, MODULUS_LIMBS_32);

impl Fr {
    pub const fn size() -> usize {
        32
    }
}

impl ff::Field for Fr {
    const ZERO: Self = Self::zero();
    const ONE: Self = Self::one();

    fn random(mut rng: impl RngCore) -> Self {
        Self::from_u512([
            rng.next_u64(),
            rng.next_u64(),
            rng.next_u64(),
            rng.next_u64(),
            rng.next_u64(),
            rng.next_u64(),
            rng.next_u64(),
            rng.next_u64(),
        ])
    }

    fn double(&self) -> Self {
        self.double()
    }

    #[inline(always)]
    fn square(&self) -> Self {
        self.square()
    }

    /// Returns the multiplicative inverse of the
    /// element. If it is zero, the method fails.
    fn invert(&self) -> CtOption<Self> {
        self.invert()
    }

    fn sqrt(&self) -> CtOption<Self> {
        /// `(t - 1) // 2` where t * 2^s + 1 = p with t odd.
        const T_MINUS1_OVER2: [u64; 4] = [
            0xcdcb848a1f0fac9f,
            0x0c0ac2e9419f4243,
            0x098d014dc2822db4,
            0x0000000183227397,
        ];
        ff::helpers::sqrt_tonelli_shanks(self, T_MINUS1_OVER2)
    }

    fn sqrt_ratio(num: &Self, div: &Self) -> (Choice, Self) {
        ff::helpers::sqrt_ratio_generic(num, div)
    }
}

impl ff::PrimeField for Fr {
    type Repr = [u8; 32];

    const NUM_BITS: u32 = 254;
    const CAPACITY: u32 = 253;
    const MODULUS: &'static str = MODULUS_STR;
    const MULTIPLICATIVE_GENERATOR: Self = GENERATOR;
    const ROOT_OF_UNITY: Self = ROOT_OF_UNITY;
    const ROOT_OF_UNITY_INV: Self = ROOT_OF_UNITY_INV;
    const TWO_INV: Self = TWO_INV;
    const DELTA: Self = DELTA;
    const S: u32 = S;

    fn from_repr(repr: Self::Repr) -> CtOption<Self> {
        let mut tmp = Fr([0, 0, 0, 0]);

        tmp.0[0] = u64::from_le_bytes(repr[0..8].try_into().unwrap());
        tmp.0[1] = u64::from_le_bytes(repr[8..16].try_into().unwrap());
        tmp.0[2] = u64::from_le_bytes(repr[16..24].try_into().unwrap());
        tmp.0[3] = u64::from_le_bytes(repr[24..32].try_into().unwrap());

        // Try to subtract the modulus
        let (_, borrow) = sbb(tmp.0[0], MODULUS.0[0], 0);
        let (_, borrow) = sbb(tmp.0[1], MODULUS.0[1], borrow);
        let (_, borrow) = sbb(tmp.0[2], MODULUS.0[2], borrow);
        let (_, borrow) = sbb(tmp.0[3], MODULUS.0[3], borrow);

        // If the element is smaller than MODULUS then the
        // subtraction will underflow, producing a borrow value
        // of 0xffff...ffff. Otherwise, it'll be zero.
        let is_some = (borrow as u8) & 1;

        // Convert to Montgomery form by computing
        // (a.R^0 * R^2) / R = a.R
        tmp *= &R2;

        CtOption::new(tmp, Choice::from(is_some))
    }

    fn to_repr(&self) -> Self::Repr {
        let tmp: [u64; 4] = (*self).into();
        let mut res = [0; 32];
        res[0..8].copy_from_slice(&tmp[0].to_le_bytes());
        res[8..16].copy_from_slice(&tmp[1].to_le_bytes());
        res[16..24].copy_from_slice(&tmp[2].to_le_bytes());
        res[24..32].copy_from_slice(&tmp[3].to_le_bytes());

        res
    }

    fn is_odd(&self) -> Choice {
        Choice::from(self.to_repr()[0] & 1)
    }
}

impl FromUniformBytes<64> for Fr {
    /// Converts a 512-bit little endian integer into
    /// an `Fr` by reducing by the modulus.
    fn from_uniform_bytes(bytes: &[u8; 64]) -> Self {
        Self::from_u512([
            u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
            u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
            u64::from_le_bytes(bytes[40..48].try_into().unwrap()),
            u64::from_le_bytes(bytes[48..56].try_into().unwrap()),
            u64::from_le_bytes(bytes[56..64].try_into().unwrap()),
        ])
    }
}

impl WithSmallOrderMulGroup<3> for Fr {
    const ZETA: Self = ZETA;
}

#[cfg(test)]
mod test {
    use super::*;
    crate::field_testing_suite!(Fr, "field_arithmetic");
    crate::field_testing_suite!(Fr, "conversion");
    crate::field_testing_suite!(Fr, "serialization");
    crate::field_testing_suite!(Fr, "quadratic_residue");
    crate::field_testing_suite!(Fr, "bits");
    crate::field_testing_suite!(Fr, "serialization_check");
    crate::field_testing_suite!(Fr, "constants", MODULUS_STR);
    crate::field_testing_suite!(Fr, "sqrt");
    crate::field_testing_suite!(Fr, "zeta");
    crate::field_testing_suite!(
        Fr,
        "from_uniform_bytes",
        [
            Fr::from_raw([
                0x2ca6366467811a07,
                0x22727e3db430ed7e,
                0xbdb79bcb97d9e250,
                0x2cee6d1152d1d7b0
            ]),
            Fr::from_raw([
                0x6ec33f1a3af8cb2d,
                0x2c8f3330e85dab4b,
                0xfeeff4ae1b019172,
                0x095cd2a455dd67b6
            ]),
            Fr::from_raw([
                0x4741eee9c02c9f33,
                0xfc0111dd8aeb7e7a,
                0xb1d79e2a22d4ab08,
                0x0cb7168893a7bbda
            ]),
            Fr::from_raw([
                0xc2ff8410555287f8,
                0x0927fbea8c6049c8,
                0xc0edccc8e4d3efe4,
                0x1d724b76911436c4
            ]),
            Fr::from_raw([
                0xdef98bc8d4db6e5b,
                0x42f0ea50590d557e,
                0x1f311a3b8114fd9a,
                0x0487c555645c67b1
            ]),
            Fr::from_raw([
                0x8ad4879b05ceb610,
                0x2e4e9a46537c84b0,
                0x5cfa7c43c9dfcfa1,
                0x0b6b2a4d122d0bb6
            ]),
            Fr::from_raw([
                0xe7f11ee016df7fe7,
                0x6419da89bd8aef3d,
                0x3511f5d293af95c8,
                0x10379c1d4d49593a
            ]),
            Fr::from_raw([
                0xd63080c8aa3ecd37,
                0x19c20f30b56fe458,
                0xc9dbbcb3aa780e06,
                0x28a4e2b8273762c6
            ]),
            Fr::from_raw([
                0xecea51b521eac0b8,
                0x65fff58a5881c562,
                0x603ac7d1e06ef3af,
                0x1e0c2c51226eecea
            ]),
            Fr::from_raw([
                0xe6ec4779b8bd6516,
                0x0d5411f3cb9504ae,
                0xff706ec73df8e92a,
                0x2c56d60b3e351e56
            ]),
        ]
    );

    #[test]
    fn bench_fr_from_u16() {
        use ark_std::{end_timer, start_timer};

        let repeat = 10000000;
        let mut rng = ark_std::test_rng();
        let base = (0..repeat).map(|_| (rng.next_u32() % (1 << 16)) as u64);

        let timer = start_timer!(|| format!("generate {repeat} Bn256 scalar field elements"));
        let _res: Vec<_> = base.map(Fr::from).collect();

        end_timer!(timer);
    }
}
