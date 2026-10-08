//! Optional LocalSend timestamps. Unknown fields are ignored by serde; malformed
//! timestamp strings do not invalidate otherwise valid content or the other timestamp.
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs::FileTimes,
    os::unix::fs::MetadataExt,
    time::{Duration, SystemTime},
};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(crate) struct Metadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    modified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    accessed: Option<String>,
}

impl Metadata {
    pub fn read(file: &std::fs::File) -> std::io::Result<Self> {
        let metadata = file.metadata()?;
        let format = |seconds, nanos: i64| {
            DateTime::<Utc>::from_timestamp(seconds, u32::try_from(nanos).ok()?)
                .map(|date| date.to_rfc3339_opts(SecondsFormat::AutoSi, true))
        };
        Ok(Self {
            modified: format(metadata.mtime(), metadata.mtime_nsec()),
            accessed: format(metadata.atime(), metadata.atime_nsec()),
        })
    }

    pub async fn apply(&self, file: &mut linuxdrop_storage::PendingFile) -> anyhow::Result<()> {
        let modified = self.modified.as_deref().and_then(parse);
        let accessed = self.accessed.as_deref().and_then(parse);
        if modified.is_none() && accessed.is_none() {
            return Ok(());
        }
        let mut times = FileTimes::new();
        if let Some(time) = modified {
            times = times.set_modified(time);
        }
        if let Some(time) = accessed {
            times = times.set_accessed(time);
        }
        file.set_times(times).await
    }
}

fn parse(text: &str) -> Option<SystemTime> {
    if text.len() > 64 {
        return None;
    }
    let date = DateTime::parse_from_rfc3339(text).ok()?;
    let seconds = date.timestamp();
    let nanos = date.timestamp_subsec_nanos();
    // Unix file timestamps cannot represent a leap second.
    if nanos >= 1_000_000_000 {
        return None;
    }
    let time = if seconds >= 0 {
        SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64))?
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(Duration::from_secs(seconds.unsigned_abs()))?
    };
    time.checked_add(Duration::from_nanos(u64::from(nanos)))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    pub(crate) fn set_test_times(path: &std::path::Path) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(
                FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + Duration::new(1609504496, 123456789))
                    .set_accessed(SystemTime::UNIX_EPOCH + Duration::new(1610000000, 987654321)),
            )
            .unwrap();
    }

    pub(crate) fn assert_test_times(path: &std::path::Path) {
        let stat = std::fs::metadata(path).unwrap();
        assert_eq!((stat.mtime(), stat.mtime_nsec()), (1609504496, 123456789));
        assert_eq!((stat.atime(), stat.atime_nsec()), (1610000000, 987654321));
    }

    #[tokio::test]
    async fn nullable_forward_fields_offsets_and_invalid_dates_preserve_only_valid_times() {
        let directory = tempfile::tempdir().unwrap();
        let store = linuxdrop_storage::ReceiveStore::open(directory.path()).unwrap();
        let metadata: Metadata = serde_json::from_value(serde_json::json!({
            "modified":"1969-12-31T23:59:59.123456789Z", "accessed":"2021-01-01T14:34:56+02:00",
            "futureField":{"anything":true}
        }))
        .unwrap();
        let mut file = store.create("dated.txt").unwrap();
        file.file.write_all(b"exact bytes").await.unwrap();
        metadata.apply(&mut file).await.unwrap();
        let path = file.commit().await.unwrap();
        let stat = std::fs::metadata(&path).unwrap();
        assert_eq!((stat.mtime(), stat.mtime_nsec()), (-1, 123456789));
        assert_eq!(stat.atime(), 1609504496);
        assert_eq!(std::fs::read(path).unwrap(), b"exact bytes");

        let metadata: Metadata = serde_json::from_value(serde_json::json!({
            "modified":"not-a-date", "accessed":null, "anotherOptionalField":123
        }))
        .unwrap();
        let mut file = store.create("undated.txt").unwrap();
        file.file.write_all(b"data").await.unwrap();
        let before = file.file.metadata().await.unwrap().modified().unwrap();
        metadata.apply(&mut file).await.unwrap();
        let path = file.commit().await.unwrap();
        assert_eq!(std::fs::metadata(path).unwrap().modified().unwrap(), before);
        assert!(parse("2021-01-01").is_none());
        assert!(parse("2016-12-31T23:59:60Z").is_none());
        assert!(parse(&"1".repeat(65)).is_none());
    }
}
