//! A byte-level BPE: tokens 0 to 255 are the bytes, [`EOS`] ends a document, and each merge
//! after it names a new token made of two before it. Any bytes encode; every token decodes.

use crate::Error;

/// The end of a document: training puts one after each, and generation stops at it.
pub const EOS: u32 = 256;
/// The first merged token.
const FIRST: u32 = 257;
/// The most bytes one token may stand for: merges of merges double, so a file's few merges
/// could otherwise ask for gigabytes.
pub const PIECE: usize = 256;

/// The bytes, [`EOS`] and the merges, in the order they were learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokenizer {
    merges: Vec<(u32, u32)>,
    pieces: Vec<Vec<u8>>,
}

impl Tokenizer {
    /// Bytes alone: 257 tokens.
    pub fn bytes() -> Tokenizer {
        let pieces = (0..=255u8).map(|b| vec![b]).chain([Vec::new()]).collect();
        Tokenizer { merges: Vec::new(), pieces }
    }

    /// The tokenizer of `merges`, each of two tokens made before it, together at most
    /// [`PIECE`] bytes.
    pub fn from_merges(merges: Vec<(u32, u32)>) -> Result<Tokenizer, Error> {
        let mut tok = Tokenizer::bytes();
        for (i, &(a, b)) in merges.iter().enumerate() {
            let made = FIRST as usize + i;
            if a as usize >= made || b as usize >= made || a == EOS || b == EOS {
                return Err(Error::Merge(i));
            }
            if tok.piece(a).len() + tok.piece(b).len() > PIECE {
                return Err(Error::Merge(i));
            }
            let piece = [tok.piece(a), tok.piece(b)].concat();
            tok.pieces.push(piece);
        }
        tok.merges = merges;
        Ok(tok)
    }

    /// Learns merges from `texts` until there are `vocab` tokens, or no pair comes twice: each
    /// time the commonest pair of neighbors (the smaller pair on a tie) whose bytes fit
    /// [`PIECE`] becomes a token. Merges never cross from one text into the next.
    pub fn train(texts: &[&str], vocab: usize) -> Tokenizer {
        let mut seqs: Vec<Vec<u32>> =
            texts.iter().map(|t| t.bytes().map(u32::from).collect()).collect();
        let mut merges = Vec::new();
        let most = vocab.max(FIRST as usize);
        let mut counts = vec![0u32; most * most];
        // Each token's bytes: 1 for a byte, 0 for the end token (never in a text).
        let mut lens: Vec<usize> = (0..FIRST).map(|t| usize::from(t != EOS)).collect();
        for id in FIRST as usize..most {
            counts[..id * most].fill(0);
            for s in &seqs {
                for w in s.windows(2) {
                    counts[w[0] as usize * most + w[1] as usize] += 1;
                }
            }
            let (mut best, mut at) = (1, None);
            for (i, &n) in counts[..id * most].iter().enumerate() {
                if n > best && lens[i / most] + lens[i % most] <= PIECE {
                    (best, at) = (n, Some(i));
                }
            }
            let Some(i) = at else { break };
            let pair = ((i / most) as u32, (i % most) as u32);
            for s in &mut seqs {
                merge(s, pair, id as u32);
            }
            lens.push(lens[i / most] + lens[i % most]);
            merges.push(pair);
        }
        Tokenizer::from_merges(merges).unwrap_or_else(|_| Tokenizer::bytes())
    }

    /// All the tokens: 257 and one per merge.
    pub fn vocab(&self) -> usize {
        self.pieces.len()
    }

    pub fn merges(&self) -> &[(u32, u32)] {
        &self.merges
    }

    /// The bytes token `id` stands for ([`EOS`] and unknown ids: none).
    pub fn piece(&self, id: u32) -> &[u8] {
        self.pieces.get(id as usize).map_or(&[], Vec::as_slice)
    }

    /// `text` as tokens: its bytes, then each merge in the order learned.
    pub fn encode(&self, text: &str) -> Vec<u32> {
        let mut seq: Vec<u32> = text.bytes().map(u32::from).collect();
        for (i, &pair) in self.merges.iter().enumerate() {
            merge(&mut seq, pair, FIRST + i as u32);
        }
        seq
    }

    /// The bytes of `ids`, as text (a cut character becomes U+FFFD).
    pub fn decode(&self, ids: &[u32]) -> String {
        let bytes: Vec<u8> = ids.iter().flat_map(|&id| self.piece(id)).copied().collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// Replaces each `pair` in `seq`, left to right, by `id`.
fn merge(seq: &mut Vec<u32>, pair: (u32, u32), id: u32) {
    let (mut r, mut w) = (0, 0);
    while r < seq.len() {
        if r + 1 < seq.len() && (seq[r], seq[r + 1]) == pair {
            seq[w] = id;
            r += 2;
        } else {
            seq[w] = seq[r];
            r += 1;
        }
        w += 1;
    }
    seq.truncate(w);
}
