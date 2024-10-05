pub mod algorithm;
pub mod dump_parser;
// it only makes sense to compare the algorithm to python if the same diff algorithm is used
#[cfg(all(test, feature = "python-diff"))]
mod integration_tests;
#[cfg(test)]
mod test_support;
pub mod utils;
