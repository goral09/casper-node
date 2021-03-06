use std::{fmt::Debug, hash::Hash};

use serde::{de::DeserializeOwned, Serialize};

/// A validator identifier.
pub(crate) trait ValidatorIdT: Eq + Ord + Clone + Debug + Hash {}
impl<VID> ValidatorIdT for VID where VID: Eq + Ord + Clone + Debug + Hash {}

/// The consensus value type, e.g. a list of transactions.
pub(crate) trait ConsensusValueT:
    Eq + Clone + Debug + Hash + Serialize + DeserializeOwned
{
}
impl<CV> ConsensusValueT for CV where CV: Eq + Clone + Debug + Hash + Serialize + DeserializeOwned {}

/// A hash, as an identifier for a block or vote.
pub(crate) trait HashT:
    Eq + Ord + Clone + Debug + Hash + Serialize + DeserializeOwned
{
}
impl<H> HashT for H where H: Eq + Ord + Clone + Debug + Hash + Serialize + DeserializeOwned {}

/// A validator's secret signing key.
pub(crate) trait ValidatorSecret {
    type Signature: Eq + Clone + Debug + Hash;

    fn sign(&self, data: &[u8]) -> Vec<u8>;
}

/// The collection of types the user can choose for cryptography, IDs, transactions, etc.
// TODO: These trait bounds make `#[derive(...)]` work for types with a `C: Context` type
// parameter. Split this up or replace the derives with explicit implementations.
pub(crate) trait Context: Clone + Debug + PartialEq {
    /// The consensus value type, e.g. a list of transactions.
    type ConsensusValue: ConsensusValueT;
    /// Unique identifiers for validators.
    type ValidatorId: ValidatorIdT;
    /// A validator's secret signing key.
    type ValidatorSecret: ValidatorSecret;
    /// Unique identifiers for votes.
    type Hash: HashT;
    /// The ID of a consensus protocol instance.
    type InstanceId: HashT;

    fn hash(data: &[u8]) -> Self::Hash;
}
