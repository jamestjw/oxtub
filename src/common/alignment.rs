// Slower but clearer equivalent:
//   fn align_up(offset: usize, align: usize) -> usize {
//       let remainder = offset % align;
//       if remainder == 0 {
//           offset
//       } else {
//           offset + (align - remainder)
//       }
//   }
pub const fn align_up(offset: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (offset + align - 1) & !(align - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_up_returns_same_offset_when_already_aligned() {
        assert_eq!(align_up(0, 8), 0);
        assert_eq!(align_up(8, 8), 8);
        assert_eq!(align_up(16, 8), 16);
    }
    #[test]
    fn align_up_rounds_up_to_next_alignment_boundary() {
        assert_eq!(align_up(1, 8), 8);
        assert_eq!(align_up(7, 8), 8);
        assert_eq!(align_up(9, 8), 16);
        assert_eq!(align_up(17, 8), 24);
    }
    #[test]
    fn align_up_with_alignment_one_is_identity() {
        assert_eq!(align_up(0, 1), 0);
        assert_eq!(align_up(123, 1), 123);
    }
    #[test]
    fn align_up_handles_page_layout_padding_case() {
        assert_eq!(align_up(22, 8), 24);
    }
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn align_up_panics_for_invalid_alignment() {
        align_up(10, 3);
    }
}
