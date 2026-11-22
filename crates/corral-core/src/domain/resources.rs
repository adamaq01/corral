use std::num::NonZeroU64;

use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allocation {
    request: NonZeroU64,
    limit: NonZeroU64,
}

impl Allocation {
    pub fn new(request: u64, limit: u64) -> Result<Self> {
        let (Some(request), Some(limit)) = (NonZeroU64::new(request), NonZeroU64::new(limit))
        else {
            return Err(Error::Invalid(
                "a resource request and limit must be positive".to_owned(),
            ));
        };
        if request > limit {
            return Err(Error::Invalid(format!(
                "a resource request of {request} is above its limit of {limit}"
            )));
        }
        Ok(Self { request, limit })
    }

    #[must_use]
    pub fn request(self) -> u64 {
        self.request.get()
    }

    #[must_use]
    pub fn limit(self) -> u64 {
        self.limit.get()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resources {
    pub cpu_millis: Allocation,
    pub memory_bytes: Allocation,
    pub home_bytes: NonZeroU64,
}

#[cfg(test)]
mod tests {
    use super::Allocation;
    use crate::Error;

    #[test]
    fn an_allocation_is_positive_and_requests_at_most_its_limit() {
        let allocation = Allocation::new(500, 2000).unwrap();
        assert_eq!((allocation.request(), allocation.limit()), (500, 2000));
        assert!(Allocation::new(7, 7).is_ok());
        for (request, limit) in [(0, 1), (1, 0), (2, 1)] {
            assert!(
                matches!(Allocation::new(request, limit), Err(Error::Invalid(_))),
                "{request} {limit}"
            );
        }
    }
}
