//! Trimming a report too large for one command: the longest head of its list
//! that fits, found in a bounded number of encodes.

use super::*;

/// A request whose encoding is `base` bytes plus each kept row's size,
/// counting how often it is encoded.
fn measured(
    base: usize,
    sizes: &[usize],
) -> (impl FnMut(usize) -> Option<String> + '_, Arc<Mutex<usize>>) {
    let encodes = Arc::new(Mutex::new(0));
    let counter = Arc::clone(&encodes);
    let encode_head = move |rows: usize| {
        *counter.lock().unwrap() += 1;
        Some("x".repeat(base + sizes[..rows].iter().sum::<usize>()))
    };
    (encode_head, encodes)
}

#[test]
fn large_leading_rows_keep_the_longest_fitting_head_in_few_encodes() {
    // A few near-maximum rows lead many small ones, the case that made the
    // old proportional trim re-encode the report about a hundred times.
    let mut sizes = vec![MAX_ENDPOINT_COMMAND_BYTES / 4 - 10_000; 3];
    sizes.extend(std::iter::repeat_n(2_000, 509));
    let base = 1_000;
    let (encode_head, encodes) = measured(base, &sizes);

    let (keep, encoded) = longest_fitting_head(sizes.len(), encode_head).expect("a head fits");

    let size = |rows: usize| base + sizes[..rows].iter().sum::<usize>();
    assert!(size(keep) <= MAX_ENDPOINT_COMMAND_BYTES);
    assert!(size(keep + 1) > MAX_ENDPOINT_COMMAND_BYTES);
    assert_eq!(encoded.len(), size(keep));
    // The whole list, the empty one, then at most ceil(log2(512)) = 9 more.
    assert!(
        *encodes.lock().unwrap() <= 11,
        "{} encodes",
        encodes.lock().unwrap()
    );
}

#[test]
fn a_report_that_fits_is_encoded_once() {
    let sizes = [10, 20, 30];
    let (encode_head, encodes) = measured(100, &sizes);

    assert_eq!(
        longest_fitting_head(sizes.len(), encode_head).map(|(keep, _)| keep),
        Some(3)
    );
    assert_eq!(*encodes.lock().unwrap(), 1);
}

#[test]
fn a_report_whose_first_row_does_not_fit_keeps_no_rows() {
    let sizes = [MAX_ENDPOINT_COMMAND_BYTES, 10];
    let (encode_head, _) = measured(100, &sizes);

    assert_eq!(
        longest_fitting_head(sizes.len(), encode_head).map(|(keep, _)| keep),
        Some(0)
    );
}

#[test]
fn a_report_without_rows_that_does_not_fit_is_not_sent() {
    let (encode_head, _) = measured(MAX_ENDPOINT_COMMAND_BYTES + 1, &[]);
    assert!(longest_fitting_head(0, encode_head).is_none());

    let sizes = [10];
    let (encode_head, _) = measured(MAX_ENDPOINT_COMMAND_BYTES + 1, &sizes);
    assert!(longest_fitting_head(sizes.len(), encode_head).is_none());
}
