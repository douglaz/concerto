use std::fmt::Debug;

use anyhow::bail;
use postgres_types::{FromSql, ToSql};
use serde::{Deserialize, Serialize};

pub const SATS_PER_BITCOIN: u64 = 100_000_000;

/// Represents an amount of BTC inside the system. The base denomination is
/// milli satoshi for now, this is also why the amount type from rust-bitcoin
/// isn't used instead.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Amount {
    pub msats: u64,
}

impl Amount {
    pub const ZERO: Self = Self { msats: 0 };

    pub const fn from_msats(msats: u64) -> Amount {
        Amount { msats }
    }

    pub const fn from_sats(sats: u64) -> Amount {
        Amount::from_msats(sats * 1000)
    }

    pub const fn from_bitcoins(bitcoins: u64) -> Amount {
        Amount::from_sats(bitcoins * SATS_PER_BITCOIN)
    }

    pub fn saturating_sub(self, other: Amount) -> Self {
        Amount {
            msats: self.msats.saturating_sub(other.msats),
        }
    }

    // Makes sure we're dealing with a precision of satoshi or higher
    pub fn ensure_sats_precision(&self) -> anyhow::Result<()> {
        if self.msats % 1000 != 0 {
            bail!("Amount is using a precision smaller than satoshi, cannot convert to satoshis");
        }
        Ok(())
    }

    pub fn try_into_sats(&self) -> anyhow::Result<u64> {
        self.ensure_sats_precision()?;
        Ok(self.msats / 1000)
    }

    pub const fn sats_round_down(&self) -> u64 {
        self.msats / 1000
    }

    pub fn sats_f64(&self) -> f64 {
        self.msats as f64 / 1000.0
    }
}

impl std::fmt::Display for Amount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} msat", self.msats)
    }
}

impl std::ops::Rem for Amount {
    type Output = Amount;

    fn rem(self, rhs: Self) -> Self::Output {
        Amount {
            msats: self.msats % rhs.msats,
        }
    }
}

impl std::ops::RemAssign for Amount {
    fn rem_assign(&mut self, rhs: Self) {
        self.msats %= rhs.msats;
    }
}

impl std::ops::Div for Amount {
    type Output = u64;

    fn div(self, rhs: Self) -> Self::Output {
        self.msats / rhs.msats
    }
}

impl std::ops::SubAssign for Amount {
    fn sub_assign(&mut self, rhs: Self) {
        self.msats -= rhs.msats
    }
}

impl std::ops::Mul<u64> for Amount {
    type Output = Amount;

    fn mul(self, rhs: u64) -> Self::Output {
        Amount {
            msats: self.msats * rhs,
        }
    }
}

impl std::ops::Mul<Amount> for u64 {
    type Output = Amount;

    fn mul(self, rhs: Amount) -> Self::Output {
        Amount {
            msats: self * rhs.msats,
        }
    }
}

impl std::ops::Add for Amount {
    type Output = Amount;

    fn add(self, rhs: Self) -> Self::Output {
        Amount {
            msats: self.msats + rhs.msats,
        }
    }
}

impl std::ops::AddAssign for Amount {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl std::iter::Sum for Amount {
    fn sum<I: Iterator<Item = Amount>>(iter: I) -> Self {
        Amount {
            msats: iter.map(|amt| amt.msats).sum::<u64>(),
        }
    }
}

impl std::ops::Sub for Amount {
    type Output = Amount;

    fn sub(self, rhs: Self) -> Self::Output {
        Amount {
            msats: self.msats - rhs.msats,
        }
    }
}

impl From<fedimint_core::Amount> for Amount {
    fn from(value: fedimint_core::Amount) -> Self {
        Self { msats: value.msats }
    }
}

impl From<Amount> for fedimint_core::Amount {
    fn from(value: Amount) -> Self {
        Self { msats: value.msats }
    }
}

impl ToSql for Amount {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        i64::try_from(self.msats)?.to_sql(ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool
    where
        Self: Sized,
    {
        <i64 as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        i64::try_from(self.msats)?.to_sql_checked(ty, out)
    }
}

impl<'a> FromSql<'a> for Amount {
    fn from_sql(
        ty: &postgres_types::Type,
        raw: &[u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let n = i64::from_sql(ty, raw)?;
        let n = u64::try_from(n)?;
        Ok(Amount::from_msats(n))
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <i64 as FromSql>::accepts(ty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amount_multiplication_by_scalar() {
        assert_eq!(Amount::from_msats(1000) * 123, Amount::from_msats(123_000));
    }

    #[test]
    fn scalar_multiplication_by_amount() {
        assert_eq!(123 * Amount::from_msats(1000), Amount::from_msats(123_000));
    }
}
