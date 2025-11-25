use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};
use num_bigint::{BigUint, ParseBigIntError};
use num_traits::Num;

#[derive(thiserror::Error, Debug, Clone)]
pub enum AssetIdError{
    #[error("Invalid base36 string")]
    InvalidBase36(#[from] ParseBigIntError),
    #[error("AssertID overflow")]
    AssertIdOverflow,
}

/// 内部存储 20 字节 (160-bit) 的哈希摘要
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId([u8; 20]);

impl AssetId {
    /// 从二进制数据生成 ID (Content-Addressable)
    pub fn from_data(data: &[u8]) -> Self {
        let hash = blake3::hash(data);
        let mut bytes = [0u8; 20];
        // 截取前 20 字节
        bytes.copy_from_slice(&hash.as_bytes()[0..20]);
        Self(bytes)
    }

    /// 获取原始字节
    pub fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }

    /// 格式化为 Base36 字符串 (带连字符)
    fn to_base36_string(&self) -> String {
        let num = BigUint::from_bytes_be(&self.0);
        let s = num.to_str_radix(36);

        // 补齐前导零到 31 位 (2^160 - 1 在 Base36 下是 31 位)
        let width = 31;
        let padded = format!("{:0>width$}", s, width = width);

        let mut formatted = String::with_capacity(36);
        formatted.push_str(&padded[0..6]);
        formatted.push('-');
        formatted.push_str(&padded[6..12]);
        formatted.push('-');
        formatted.push_str(&padded[12..18]);
        formatted.push('-');
        formatted.push_str(&padded[18..24]);
        formatted.push('-');
        formatted.push_str(&padded[24..31]);

        formatted
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_base36_string())
    }
}

impl fmt::Debug for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssetId({})", self.to_base36_string())
    }
}

impl FromStr for AssetId {
    type Err = AssetIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let clean = s.trim().trim_start_matches("asset-").trim_start_matches("asset/");
        let clean = clean.trim().trim_start_matches("image-").trim_start_matches("image/");

        let raw = clean.replace('-', "");

        let num = BigUint::from_str_radix(&raw, 36).map_err(|e| AssetIdError::InvalidBase36(e) )?;

        let bytes = num.to_bytes_be();

        if bytes.len() > 20 {
            return Err(AssetIdError::AssertIdOverflow);
        }

        let mut arr = [0u8; 20];
        // BigUint 转 bytes 可能会丢弃前导零，需要从后往前填
        let start = 20 - bytes.len();
        arr[start..].copy_from_slice(&bytes);

        Ok(Self(arr))
    }
}

impl Serialize for AssetId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for AssetId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        AssetId::from_str(&s).map_err(serde::de::Error::custom)
    }
}

impl AsRef<[u8]> for AssetId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
