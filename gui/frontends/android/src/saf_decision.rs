//! Actual SAF stream collection, host-tested: only an empty chunk is EOF.
//! Errors discard the partial buffer; short chunks are data, not EOF.

pub(crate) fn collect_stream<E>(
    mut read: impl FnMut() -> Result<Vec<u8>, E>,
) -> Result<Vec<u8>, E> {
    let mut bytes = Vec::new();
    loop {
        let chunk = read()?;
        if chunk.is_empty() {
            return Ok(bytes);
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
