//! field-hash <dogTagIdDecimal> — prints field_of_value(Integer(dec)) as a decimal uint256.
//! This is the CANONICAL on-chain dogTagId: the DOG_PROFILE SBT must be minted with THIS value (so
//! `ownerOf` matches the circuit's `pub[0]`), and the §1.10 consent / EdDSA message / nullifier use it
//! too. The raw numeric is only an off-chain handle (what the operator types into the vaccination form).
use ark_ff::PrimeField;
use dogtag_standard::leaf::field_of_value;
use dogtag_standard::types::TypeTag;
use dogtag_standard::wrap::scalar_from_packed;

fn main() {
    let arg = std::env::args()
        .nth(1)
        .expect("usage: field-hash <dogTagIdDecimal>");
    let scalar = scalar_from_packed(TypeTag::Integer, arg.trim())
        .unwrap_or_else(|e| panic!("scalar_from_packed: {e}"));
    let f = field_of_value(&scalar).unwrap_or_else(|e| panic!("field_of_value: {e}"));
    println!("{}", f.into_bigint());
}
