//! Actual SAF stream collection, host-tested: only an empty chunk is EOF.
//! Errors discard the partial buffer; short chunks are data, not EOF.

pub(crate) fn collect_stream<E>(read: impl FnMut() -> Result<Vec<u8>, E>) -> Result<Vec<u8>, E> {
    collect_stream_capped(read, usize::MAX).map_err(|error| match error {
        CollectError::Read(error) => error,
        CollectError::OverLimit { .. } => unreachable!("unbounded collect cannot exceed"),
    })
}

/// Bounded stream error: reads fail as `Read`, oversize as `OverLimit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CollectError<E> {
    Read(E),
    OverLimit { limit: usize },
}

/// Capped collection: unknown-size providers cannot OOM the worker.
/// Past `limit` the partial buffer drops and `OverLimit` reports.
pub(crate) fn collect_stream_capped<E>(
    mut read: impl FnMut() -> Result<Vec<u8>, E>,
    limit: usize,
) -> Result<Vec<u8>, CollectError<E>> {
    let mut bytes = Vec::new();
    loop {
        let chunk = read().map_err(CollectError::Read)?;
        if chunk.is_empty() {
            return Ok(bytes);
        }
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(CollectError::OverLimit { limit });
        }
        bytes.extend_from_slice(&chunk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_chunks_are_data_until_explicit_eof() {
        let mut chunks = [Ok(vec![1, 2]), Ok(vec![3]), Ok(vec![])].into_iter();
        assert_eq!(
            collect_stream::<()>(|| chunks.next().unwrap()),
            Ok(vec![1, 2, 3])
        );
        assert!(chunks.next().is_none());
    }

    #[test]
    fn error_after_data_never_returns_truncated_success() {
        let mut chunks = [Ok(vec![1, 2]), Err("read failed")].into_iter();
        assert_eq!(
            collect_stream(|| chunks.next().unwrap()),
            Err("read failed")
        );
    }

    #[test]
    fn empty_file_and_immediate_error_are_distinct() {
        assert_eq!(collect_stream::<()>(|| Ok(vec![])), Ok(vec![]));
        assert_eq!(
            collect_stream(|| Err("invalid session")),
            Err("invalid session")
        );
    }
}

#[cfg(test)]
mod capped_tests {
    use super::*;

    #[test]
    fn capped_collect_rejects_past_limit_without_partial_success() {
        let mut chunks = [Ok(vec![1, 2, 3]), Ok(vec![4])].into_iter();
        assert_eq!(
            collect_stream_capped::<()>(|| chunks.next().unwrap(), 3),
            Err(CollectError::OverLimit { limit: 3 })
        );
    }

    #[test]
    fn capped_collect_accepts_at_limit() {
        let mut chunks = [Ok(vec![1, 2]), Ok(vec![3]), Ok(vec![])].into_iter();
        assert_eq!(
            collect_stream_capped::<()>(|| chunks.next().unwrap(), 3),
            Ok(vec![1, 2, 3])
        );
    }
}
