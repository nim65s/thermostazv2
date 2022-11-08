use bincode::{Decode, Encode};

#[derive(Encode, Decode, Debug, Eq, PartialEq)]
pub struct A {
    pub stop: bool,
    pub pose: u16,
}

#[derive(Encode, Decode, Debug, Eq, PartialEq)]
pub struct B {
    pub goal: A,
    pub meas: A,
}

#[derive(Encode, Decode, Debug, Eq, PartialEq)]
pub enum C {
    A(A),
    B(B),
}
