use super::*;

// OneDrive's QuickXorHash (the `file.hashes.quickXorHash` Graph returns for
// every file): a 160-bit XOR-and-shift digest with the content length XORed
// into the last 8 bytes, base64 encoded. Used to match identical files at
// first sync and to verify downloads. Port of Microsoft's reference
// implementation.

pub(crate) const QUICK_XOR_WIDTH_BITS: usize = 160;
pub(crate) const QUICK_XOR_SHIFT: usize = 11;
pub(crate) const QUICK_XOR_CELLS: usize = (QUICK_XOR_WIDTH_BITS - 1) / 64 + 1;
pub(crate) const QUICK_XOR_BITS_IN_LAST_CELL: usize = 32;

pub(crate) struct QuickXorHasher {
    pub(crate) cells: [u64; QUICK_XOR_CELLS],
    pub(crate) shift_so_far: usize,
    pub(crate) length_so_far: u64,
}

impl QuickXorHasher {
    pub(crate) fn new() -> Self {
        Self {
            cells: [0; QUICK_XOR_CELLS],
            shift_so_far: 0,
            length_so_far: 0,
        }
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        let mut cell_index = self.shift_so_far / 64;
        let mut cell_offset = self.shift_so_far % 64;
        let iterations = bytes.len().min(QUICK_XOR_WIDTH_BITS);

        for start in 0..iterations {
            let is_last_cell = cell_index == QUICK_XOR_CELLS - 1;
            let bits_in_cell = if is_last_cell {
                QUICK_XOR_BITS_IN_LAST_CELL
            } else {
                64
            };

            if cell_offset <= bits_in_cell - 8 {
                let mut position = start;
                while position < bytes.len() {
                    self.cells[cell_index] ^= u64::from(bytes[position]) << cell_offset;
                    position += QUICK_XOR_WIDTH_BITS;
                }
            } else {
                let next_cell_index = if is_last_cell { 0 } else { cell_index + 1 };
                let low_bits = bits_in_cell - cell_offset;
                let mut xored_byte: u8 = 0;
                let mut position = start;
                while position < bytes.len() {
                    xored_byte ^= bytes[position];
                    position += QUICK_XOR_WIDTH_BITS;
                }
                self.cells[cell_index] ^= u64::from(xored_byte) << cell_offset;
                self.cells[next_cell_index] ^= u64::from(xored_byte) >> low_bits;
            }

            cell_offset += QUICK_XOR_SHIFT;
            while cell_offset >= bits_in_cell {
                cell_index = if is_last_cell { 0 } else { cell_index + 1 };
                cell_offset -= bits_in_cell;
            }
        }

        self.shift_so_far = (self.shift_so_far
            + QUICK_XOR_SHIFT * (bytes.len() % QUICK_XOR_WIDTH_BITS))
            % QUICK_XOR_WIDTH_BITS;
        self.length_so_far += bytes.len() as u64;
    }

    pub(crate) fn finish_base64(&self) -> String {
        use base64::Engine;
        let mut digest = [0_u8; QUICK_XOR_WIDTH_BITS / 8];
        for (cell_number, cell) in self.cells.iter().enumerate() {
            let cell_bytes = cell.to_le_bytes();
            let start = cell_number * 8;
            let end = (start + 8).min(digest.len());
            digest[start..end].copy_from_slice(&cell_bytes[..end - start]);
        }
        let length_bytes = self.length_so_far.to_le_bytes();
        let length_start = digest.len() - length_bytes.len();
        for (offset, length_byte) in length_bytes.iter().enumerate() {
            digest[length_start + offset] ^= length_byte;
        }
        base64::engine::general_purpose::STANDARD.encode(digest)
    }
}

/// The QuickXorHash of a local file, base64 encoded like Graph's value.
pub(crate) fn quick_xor_hash_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("Failed opening '{}' for hashing: {error}", path.display()))?;
    let mut hasher = QuickXorHasher::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Failed reading '{}' for hashing: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finish_base64())
}

#[cfg(test)]
mod quick_xor_hash_tests {
    use super::*;

    fn hash_of(bytes: &[u8]) -> String {
        let mut hasher = QuickXorHasher::new();
        hasher.update(bytes);
        hasher.finish_base64()
    }

    #[test]
    fn empty_input_matches_graph_value() {
        assert_eq!(hash_of(b""), "AAAAAAAAAAAAAAAAAAAAAAAAAAA=");
    }

    #[test]
    fn chunked_updates_match_a_single_update() {
        let content: Vec<u8> = (0..10_000_u32)
            .map(|value| (value * 31 % 251) as u8)
            .collect();
        let whole = hash_of(&content);
        for chunk_size in [1, 7, 159, 160, 161, 4096] {
            let mut hasher = QuickXorHasher::new();
            for chunk in content.chunks(chunk_size) {
                hasher.update(chunk);
            }
            assert_eq!(hasher.finish_base64(), whole, "chunk size {chunk_size}");
        }
    }

    #[test]
    fn different_content_gives_different_hashes() {
        assert_ne!(hash_of(b"hello world"), hash_of(b"hello worle"));
        assert_ne!(hash_of(b"a"), hash_of(b"aa"));
    }
}
