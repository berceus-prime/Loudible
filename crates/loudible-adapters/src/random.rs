use ring::rand::{SecureRandom, SystemRandom};
use uuid::Uuid;

/// Randomness for sign-in identities. Tests replace it to get predictable values.
pub trait RandomSource: Send + Sync {
    fn bytes(&self, count: usize) -> Vec<u8>;
    fn uuid(&self) -> Uuid;
}

pub struct SystemRandomSource;

impl RandomSource for SystemRandomSource {
    fn bytes(&self, count: usize) -> Vec<u8> {
        let mut bytes = vec![0; count];
        SystemRandom::new()
            .fill(&mut bytes)
            .expect("the system random number generator is available");
        bytes
    }

    fn uuid(&self) -> Uuid {
        Uuid::new_v4()
    }
}
