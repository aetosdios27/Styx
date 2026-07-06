//! Strict BEP 3 bencode encoding and decoding.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::ops::Range;

use bytes::{BufMut, Bytes, BytesMut};

const MAX_NESTING_DEPTH: usize = 128;
const MAX_BYTE_STRING_LENGTH: usize = 32 * 1024 * 1024;

/// A decoded BEP 3 bencode value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BencodeValue {
    /// A signed base-10 integer.
    Integer(i64),
    /// An arbitrary byte string.
    Bytes(Bytes),
    /// An ordered sequence of values.
    List(Vec<BencodeValue>),
    /// A dictionary with lexicographically sorted byte-string keys.
    Dict(BTreeMap<Vec<u8>, BencodeValue>),
}

/// A decoded value together with the byte range it occupied in the source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedBencode {
    /// Decoded value.
    pub value: BencodeValue,
    /// Half-open byte range in the original input.
    pub span: Range<usize>,
}

/// Errors returned while decoding bencode data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BencodeError {
    /// The input contained no top-level value.
    EmptyInput,
    /// Extra bytes remained after a complete top-level value.
    TrailingBytes { offset: usize },
    /// A value started with an invalid byte.
    InvalidToken { offset: usize, byte: u8 },
    /// The parser reached the end of input before a value was complete.
    UnexpectedEof { offset: usize },
    /// An integer was not in canonical BEP 3 form.
    InvalidInteger { offset: usize },
    /// An integer did not fit in `i64`.
    IntegerOverflow { offset: usize },
    /// A byte string length was malformed or non-canonical.
    InvalidByteStringLength { offset: usize },
    /// A byte string declared more bytes than the input contains.
    ByteStringOutOfBounds {
        offset: usize,
        length: usize,
        remaining: usize,
    },
    /// A dictionary key was not a byte string.
    InvalidDictionaryKey { offset: usize },
    /// Dictionary keys were not strictly lexicographically sorted.
    UnsortedDictionaryKey { offset: usize },
    /// A dictionary repeated the same key.
    DuplicateDictionaryKey { offset: usize },
    /// The nesting depth exceeded the parser limit.
    DepthLimitExceeded { offset: usize, limit: usize },
    /// A byte string length exceeded the maximum allowed size.
    ByteStringTooLarge { offset: usize, limit: usize },
}

impl fmt::Display for BencodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyInput => write!(f, "bencode input is empty"),
            Self::TrailingBytes { offset } => {
                write!(f, "trailing bytes after bencode value at offset {offset}")
            }
            Self::InvalidToken { offset, byte } => {
                write!(f, "invalid bencode token 0x{byte:02x} at offset {offset}")
            }
            Self::UnexpectedEof { offset } => {
                write!(f, "unexpected end of bencode input at offset {offset}")
            }
            Self::InvalidInteger { offset } => {
                write!(f, "invalid bencode integer at offset {offset}")
            }
            Self::IntegerOverflow { offset } => {
                write!(f, "bencode integer overflows i64 at offset {offset}")
            }
            Self::InvalidByteStringLength { offset } => {
                write!(f, "invalid bencode byte string length at offset {offset}")
            }
            Self::ByteStringOutOfBounds {
                offset,
                length,
                remaining,
            } => write!(
                f,
                "byte string at offset {offset} declares {length} bytes with only {remaining} remaining"
            ),
            Self::InvalidDictionaryKey { offset } => {
                write!(f, "dictionary key at offset {offset} is not a byte string")
            }
            Self::UnsortedDictionaryKey { offset } => {
                write!(f, "dictionary key at offset {offset} is out of order")
            }
            Self::DuplicateDictionaryKey { offset } => {
                write!(f, "duplicate dictionary key at offset {offset}")
            }
            Self::DepthLimitExceeded { offset, limit } => {
                write!(f, "bencode nesting exceeds limit {limit} at offset {offset}")
            }
            Self::ByteStringTooLarge { offset, limit } => {
                write!(
                    f,
                    "byte string at offset {offset} exceeds maximum length {limit}"
                )
            }
        }
    }
}

impl Error for BencodeError {}

/// Decode exactly one bencode value from `input`.
///
/// # Errors
///
/// Returns [`BencodeError`] when input is malformed, non-canonical, too deeply
/// nested, or contains trailing bytes after the top-level value.
pub fn decode(input: &[u8]) -> Result<BencodeValue, BencodeError> {
    decode_with_span(input).map(|decoded| decoded.value)
}

/// Decode exactly one bencode value and return its source byte range.
///
/// # Errors
///
/// Returns [`BencodeError`] when input is malformed, non-canonical, too deeply
/// nested, or contains trailing bytes after the top-level value.
pub fn decode_with_span(input: &[u8]) -> Result<DecodedBencode, BencodeError> {
    if input.is_empty() {
        return Err(BencodeError::EmptyInput);
    }

    let mut parser = Parser { input, offset: 0 };
    let decoded = parser.parse_value(0)?;
    if parser.offset != input.len() {
        return Err(BencodeError::TrailingBytes {
            offset: parser.offset,
        });
    }
    Ok(decoded)
}

/// Encode a bencode value using canonical BEP 3 dictionary key ordering.
#[must_use]
pub fn encode(value: &BencodeValue) -> Vec<u8> {
    let mut output = BytesMut::new();
    encode_into(value, &mut output);
    output.to_vec()
}

fn encode_into(value: &BencodeValue, output: &mut BytesMut) {
    match value {
        BencodeValue::Integer(integer) => {
            output.put_u8(b'i');
            output.extend_from_slice(integer.to_string().as_bytes());
            output.put_u8(b'e');
        }
        BencodeValue::Bytes(bytes) => {
            output.extend_from_slice(bytes.len().to_string().as_bytes());
            output.put_u8(b':');
            output.extend_from_slice(bytes);
        }
        BencodeValue::List(values) => {
            output.put_u8(b'l');
            for item in values {
                encode_into(item, output);
            }
            output.put_u8(b'e');
        }
        BencodeValue::Dict(values) => {
            output.put_u8(b'd');
            for (key, value) in values {
                output.extend_from_slice(key.len().to_string().as_bytes());
                output.put_u8(b':');
                output.extend_from_slice(key);
                encode_into(value, output);
            }
            output.put_u8(b'e');
        }
    }
}

struct Parser<'a> {
    input: &'a [u8],
    offset: usize,
}

impl Parser<'_> {
    fn parse_value(&mut self, depth: usize) -> Result<DecodedBencode, BencodeError> {
        if depth > MAX_NESTING_DEPTH {
            return Err(BencodeError::DepthLimitExceeded {
                offset: self.offset,
                limit: MAX_NESTING_DEPTH,
            });
        }

        let Some(byte) = self.peek() else {
            return Err(BencodeError::UnexpectedEof {
                offset: self.offset,
            });
        };

        match byte {
            b'i' => self.parse_integer(),
            b'l' => self.parse_list(depth),
            b'd' => self.parse_dict(depth),
            b'0'..=b'9' => {
                let start = self.offset;
                let bytes = self.parse_bytes()?;
                Ok(DecodedBencode {
                    value: BencodeValue::Bytes(bytes),
                    span: start..self.offset,
                })
            }
            byte => Err(BencodeError::InvalidToken {
                offset: self.offset,
                byte,
            }),
        }
    }

    fn parse_integer(&mut self) -> Result<DecodedBencode, BencodeError> {
        let start = self.offset;
        self.offset += 1;
        let digits_start = self.offset;

        while matches!(self.peek(), Some(b'0'..=b'9' | b'-')) {
            self.offset += 1;
        }

        if self.peek() != Some(b'e') {
            return if self.offset >= self.input.len() {
                Err(BencodeError::UnexpectedEof {
                    offset: self.offset,
                })
            } else {
                Err(BencodeError::InvalidInteger { offset: start })
            };
        }

        let token = &self.input[digits_start..self.offset];
        self.offset += 1;
        let integer = parse_canonical_i64(token, start)?;
        Ok(DecodedBencode {
            value: BencodeValue::Integer(integer),
            span: start..self.offset,
        })
    }

    fn parse_bytes(&mut self) -> Result<Bytes, BencodeError> {
        let start = self.offset;
        let length = self.parse_byte_string_length()?;
        if length > MAX_BYTE_STRING_LENGTH {
            return Err(BencodeError::ByteStringTooLarge {
                offset: start,
                limit: MAX_BYTE_STRING_LENGTH,
            });
        }
        let data_start = self.offset;
        let remaining = self.input.len().saturating_sub(data_start);
        if remaining < length {
            return Err(BencodeError::ByteStringOutOfBounds {
                offset: start,
                length,
                remaining,
            });
        }

        let data_end = data_start + length;
        self.offset = data_end;
        Ok(Bytes::copy_from_slice(&self.input[data_start..data_end]))
    }

    fn parse_byte_string_length(&mut self) -> Result<usize, BencodeError> {
        let start = self.offset;
        let digits_start = self.offset;

        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.offset += 1;
        }

        if self.peek() != Some(b':') {
            return if self.offset >= self.input.len() {
                Err(BencodeError::UnexpectedEof {
                    offset: self.offset,
                })
            } else {
                Err(BencodeError::InvalidByteStringLength { offset: start })
            };
        }

        let digits = &self.input[digits_start..self.offset];
        if digits.is_empty() || (digits.len() > 1 && digits[0] == b'0') {
            return Err(BencodeError::InvalidByteStringLength { offset: start });
        }

        let mut length = 0usize;
        for digit in digits {
            length = length
                .checked_mul(10)
                .and_then(|value| value.checked_add(usize::from(digit - b'0')))
                .ok_or(BencodeError::InvalidByteStringLength { offset: start })?;
        }

        self.offset += 1;
        Ok(length)
    }

    fn parse_list(&mut self, depth: usize) -> Result<DecodedBencode, BencodeError> {
        let start = self.offset;
        self.offset += 1;
        let mut values = Vec::new();

        loop {
            match self.peek() {
                Some(b'e') => {
                    self.offset += 1;
                    return Ok(DecodedBencode {
                        value: BencodeValue::List(values),
                        span: start..self.offset,
                    });
                }
                Some(_) => values.push(self.parse_value(depth + 1)?.value),
                None => {
                    return Err(BencodeError::UnexpectedEof {
                        offset: self.offset,
                    })
                }
            }
        }
    }

    fn parse_dict(&mut self, depth: usize) -> Result<DecodedBencode, BencodeError> {
        let start = self.offset;
        self.offset += 1;
        let mut values = BTreeMap::new();
        let mut previous_key: Option<Vec<u8>> = None;

        loop {
            match self.peek() {
                Some(b'e') => {
                    self.offset += 1;
                    return Ok(DecodedBencode {
                        value: BencodeValue::Dict(values),
                        span: start..self.offset,
                    });
                }
                Some(b'0'..=b'9') => {
                    let key_offset = self.offset;
                    let key = self.parse_bytes()?.to_vec();
                    if let Some(previous) = previous_key.as_ref() {
                        match key.as_slice().cmp(previous.as_slice()) {
                            std::cmp::Ordering::Less => {
                                return Err(BencodeError::UnsortedDictionaryKey {
                                    offset: key_offset,
                                })
                            }
                            std::cmp::Ordering::Equal => {
                                return Err(BencodeError::DuplicateDictionaryKey {
                                    offset: key_offset,
                                })
                            }
                            std::cmp::Ordering::Greater => {}
                        }
                    }

                    let value = self.parse_value(depth + 1)?.value;
                    previous_key = Some(key.clone());
                    values.insert(key, value);
                }
                Some(_) => {
                    return Err(BencodeError::InvalidDictionaryKey {
                        offset: self.offset,
                    })
                }
                None => {
                    return Err(BencodeError::UnexpectedEof {
                        offset: self.offset,
                    })
                }
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.offset).copied()
    }
}

pub(crate) struct SpannedDictEntry {
    pub key: Vec<u8>,
    pub value: BencodeValue,
    pub value_span: Range<usize>,
}

pub(crate) fn decode_top_level_dict_entries(
    input: &[u8],
) -> Result<Vec<SpannedDictEntry>, BencodeError> {
    if input.is_empty() {
        return Err(BencodeError::EmptyInput);
    }

    let mut parser = Parser { input, offset: 0 };
    let entries = parser.parse_top_level_dict_entries()?;
    if parser.offset != input.len() {
        return Err(BencodeError::TrailingBytes {
            offset: parser.offset,
        });
    }
    Ok(entries)
}

impl Parser<'_> {
    fn parse_top_level_dict_entries(&mut self) -> Result<Vec<SpannedDictEntry>, BencodeError> {
        if self.peek() != Some(b'd') {
            let byte = self.peek().ok_or(BencodeError::UnexpectedEof {
                offset: self.offset,
            })?;
            return Err(BencodeError::InvalidToken {
                offset: self.offset,
                byte,
            });
        }

        self.offset += 1;
        let mut entries = Vec::new();
        let mut previous_key: Option<Vec<u8>> = None;

        loop {
            match self.peek() {
                Some(b'e') => {
                    self.offset += 1;
                    return Ok(entries);
                }
                Some(b'0'..=b'9') => {
                    let key_offset = self.offset;
                    let key = self.parse_bytes()?.to_vec();
                    if let Some(previous) = previous_key.as_ref() {
                        match key.as_slice().cmp(previous.as_slice()) {
                            std::cmp::Ordering::Less => {
                                return Err(BencodeError::UnsortedDictionaryKey {
                                    offset: key_offset,
                                })
                            }
                            std::cmp::Ordering::Equal => {
                                return Err(BencodeError::DuplicateDictionaryKey {
                                    offset: key_offset,
                                })
                            }
                            std::cmp::Ordering::Greater => {}
                        }
                    }

                    let decoded = self.parse_value(1)?;
                    previous_key = Some(key.clone());
                    entries.push(SpannedDictEntry {
                        key,
                        value: decoded.value,
                        value_span: decoded.span,
                    });
                }
                Some(_) => {
                    return Err(BencodeError::InvalidDictionaryKey {
                        offset: self.offset,
                    })
                }
                None => {
                    return Err(BencodeError::UnexpectedEof {
                        offset: self.offset,
                    })
                }
            }
        }
    }
}

fn parse_canonical_i64(token: &[u8], offset: usize) -> Result<i64, BencodeError> {
    if token.is_empty() {
        return Err(BencodeError::InvalidInteger { offset });
    }

    let (negative, digits) = if token[0] == b'-' {
        (true, &token[1..])
    } else {
        (false, token)
    };

    if digits.is_empty()
        || digits.iter().any(|byte| !byte.is_ascii_digit())
        || (digits.len() > 1 && digits[0] == b'0')
        || (negative && digits == b"0")
    {
        return Err(BencodeError::InvalidInteger { offset });
    }

    let limit = if negative {
        9_223_372_036_854_775_808u64
    } else {
        i64::MAX as u64
    };
    let mut magnitude = 0u64;
    for digit in digits {
        magnitude = magnitude
            .checked_mul(10)
            .and_then(|value| value.checked_add(u64::from(digit - b'0')))
            .ok_or(BencodeError::IntegerOverflow { offset })?;
        if magnitude > limit {
            return Err(BencodeError::IntegerOverflow { offset });
        }
    }

    if negative {
        if magnitude == limit {
            Ok(i64::MIN)
        } else {
            Ok(-(magnitude as i64))
        }
    } else {
        Ok(magnitude as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn bytes(value: &[u8]) -> BencodeValue {
        BencodeValue::Bytes(Bytes::copy_from_slice(value))
    }

    #[test]
    fn decode_accepts_canonical_integers() {
        assert_eq!(decode(b"i0e"), Ok(BencodeValue::Integer(0)));
        assert_eq!(decode(b"i42e"), Ok(BencodeValue::Integer(42)));
        assert_eq!(decode(b"i-42e"), Ok(BencodeValue::Integer(-42)));
    }

    #[test]
    fn decode_rejects_integer_leading_zero() {
        assert_eq!(
            decode(b"i03e"),
            Err(BencodeError::InvalidInteger { offset: 0 })
        );
    }

    #[test]
    fn decode_rejects_negative_zero() {
        assert_eq!(
            decode(b"i-0e"),
            Err(BencodeError::InvalidInteger { offset: 0 })
        );
    }

    #[test]
    fn decode_rejects_unterminated_integer() {
        assert_eq!(
            decode(b"i42"),
            Err(BencodeError::UnexpectedEof { offset: 3 })
        );
    }

    #[test]
    fn decode_rejects_integer_overflow() {
        assert_eq!(
            decode(b"i9223372036854775808e"),
            Err(BencodeError::IntegerOverflow { offset: 0 })
        );
    }

    #[test]
    fn decode_accepts_byte_strings() {
        assert_eq!(decode(b"0:"), Ok(bytes(b"")));
        assert_eq!(decode(b"4:spam"), Ok(bytes(b"spam")));
    }

    #[test]
    fn decode_rejects_byte_string_length_leading_zero() {
        assert_eq!(
            decode(b"03:abc"),
            Err(BencodeError::InvalidByteStringLength { offset: 0 })
        );
    }

    #[test]
    fn decode_rejects_short_byte_string_input() {
        assert_eq!(
            decode(b"4:abc"),
            Err(BencodeError::ByteStringOutOfBounds {
                offset: 0,
                length: 4,
                remaining: 3,
            })
        );
    }

    #[test]
    fn decode_accepts_lists_and_dictionaries() {
        let mut dict = BTreeMap::new();
        dict.insert(b"bar".to_vec(), BencodeValue::Integer(42));
        dict.insert(b"foo".to_vec(), bytes(b"spam"));

        assert_eq!(
            decode(b"d3:bari42e3:foo4:spame"),
            Ok(BencodeValue::Dict(dict))
        );
        assert_eq!(
            decode(b"li1e4:spame"),
            Ok(BencodeValue::List(vec![
                BencodeValue::Integer(1),
                bytes(b"spam")
            ]))
        );
    }

    #[test]
    fn decode_rejects_unsorted_dictionary_keys() {
        assert_eq!(
            decode(b"d3:foo4:spam3:bari42ee"),
            Err(BencodeError::UnsortedDictionaryKey { offset: 12 })
        );
    }

    #[test]
    fn decode_rejects_duplicate_dictionary_keys() {
        assert_eq!(
            decode(b"d3:fooi1e3:fooi2ee"),
            Err(BencodeError::DuplicateDictionaryKey { offset: 9 })
        );
    }

    #[test]
    fn decode_rejects_trailing_bytes() {
        assert_eq!(
            decode(b"i1ei2e"),
            Err(BencodeError::TrailingBytes { offset: 3 })
        );
    }

    #[test]
    fn encode_orders_dictionary_keys() {
        let mut dict = BTreeMap::new();
        dict.insert(b"foo".to_vec(), bytes(b"spam"));
        dict.insert(b"bar".to_vec(), BencodeValue::Integer(42));

        assert_eq!(encode(&BencodeValue::Dict(dict)), b"d3:bari42e3:foo4:spame");
    }

    #[test]
    fn nested_values_round_trip() {
        let mut dict = BTreeMap::new();
        dict.insert(
            b"list".to_vec(),
            BencodeValue::List(vec![BencodeValue::Integer(-3), bytes(b"abc")]),
        );
        dict.insert(b"zero".to_vec(), BencodeValue::Integer(0));
        let value = BencodeValue::Dict(dict);

        assert_eq!(decode(&encode(&value)), Ok(value));
    }

    #[test]
    fn decode_rejects_excessive_nesting() {
        let mut input = vec![b'l'; MAX_NESTING_DEPTH + 2];
        input.extend(std::iter::repeat_n(b'e', MAX_NESTING_DEPTH + 2));

        assert_eq!(
            decode(&input),
            Err(BencodeError::DepthLimitExceeded {
                offset: MAX_NESTING_DEPTH + 1,
                limit: MAX_NESTING_DEPTH,
            })
        );
    }

    #[test]
    fn decode_rejects_large_byte_string() {
        let limit = 32 * 1024 * 1024;
        // A byte string length of limit+1 with no actual data
        let input = format!("{}:", limit + 1);
        assert_eq!(
            decode(input.as_bytes()),
            Err(BencodeError::ByteStringTooLarge { offset: 0, limit })
        );
    }

    #[test]
    fn decode_rejects_deeply_nested_list() {
        let depth = 130;
        let mut input = vec![b'l'; depth];
        input.extend(std::iter::repeat_n(b'e', depth));

        assert_eq!(
            decode(&input),
            Err(BencodeError::DepthLimitExceeded {
                offset: 129,
                limit: 128,
            })
        );
    }

    #[test]
    fn decode_top_level_dict_entries_preserves_value_span() {
        let entries = decode_top_level_dict_entries(b"d4:infodee").unwrap();
        assert_eq!(entries[0].key, b"info");
        assert_eq!(entries[0].value_span, 7..9);
    }

    fn arb_bencode_value() -> impl Strategy<Value = BencodeValue> {
        let leaf = prop_oneof![
            any::<i64>().prop_map(BencodeValue::Integer),
            prop::collection::vec(any::<u8>(), 0..32)
                .prop_map(|value| BencodeValue::Bytes(Bytes::from(value))),
        ];

        leaf.prop_recursive(8, 64, 8, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..8).prop_map(BencodeValue::List),
                prop::collection::btree_map(prop::collection::vec(any::<u8>(), 0..16), inner, 0..8)
                    .prop_map(BencodeValue::Dict),
            ]
        })
    }

    proptest! {
        #[test]
        fn encode_decode_round_trip(value in arb_bencode_value()) {
            prop_assert_eq!(decode(&encode(&value)), Ok(value));
        }

        #[test]
        fn decode_never_panics_for_arbitrary_input(input in prop::collection::vec(any::<u8>(), 0..512)) {
            let _ = decode(&input);
        }
    }
}
