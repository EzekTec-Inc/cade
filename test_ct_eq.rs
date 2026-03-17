use subtle::ConstantTimeEq;
fn main() {
    let a = b"123";
    let b = b"1234";
    let _ = a.ct_eq(b);
}
