use rocket::response::{self, Responder, Response};
use std::cell::RefCell;
use std::io::{SeekFrom};
use std::path::Path;
use futures::executor::block_on;
use rocket::tokio::fs::File;
use rocket::tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt};
use tree_magic;

/// Alias trait for Read + Seek
pub trait ReadSeek: AsyncRead + AsyncSeek + Unpin + Send {}
impl<T: AsyncRead + AsyncSeek + Unpin + Send> ReadSeek for T {}

/// Infer the mime type of a stream of bytes using an excerpt from the beginning of the stream
fn infer_mime_type(prelude: &[u8]) -> String {
    return tree_magic::from_u8(prelude);
}

/// Serves a readable and seekable stream,
/// The mime type can optionally be inferred by taking a sample of
/// bytes from the beginning of the stream.
///
/// The Accept Ranges header will always be set.
/// If a range request is received, it will respond with the requested offset.
/// Multipart range requests are also supported.
pub struct SeekStream {
    stream: RefCell<Box<dyn ReadSeek>>,
    length: Option<u64>,
    content_type: Option<&'static str>,
}

impl SeekStream {
    pub fn new<T>(s: T) -> SeekStream
    where
        T: AsyncRead + AsyncSeek + Unpin + Send + 'static,
    {
        Self::with_opts(s, None, None)
    }

    pub fn with_opts<T>(
        stream: T,
        stream_len: impl Into<Option<u64>>,
        content_type: impl Into<Option<&'static str>>,
    ) -> SeekStream
    where
        T: AsyncRead + AsyncSeek + Unpin + Send + 'static,
    {
        SeekStream {
            stream: RefCell::new(Box::new(stream)),
            length: stream_len.into(),
            content_type: content_type.into(),
        }
    }

    /// Serve content from a file path. The mime type will be inferred by taking a sample from
    /// The beginning of the stream.
    pub async fn from_path<T: AsRef<Path>>(p: T) -> std::io::Result<SeekStream> {
        let mut file = File::open(p.as_ref()).await?;
        let len = file.seek(SeekFrom::End(0)).await?;

        Ok(Self::with_opts(file, len, None))
    }
}

// Convert a range to a satisfiable range
fn to_satisfiable_range(
    from: Option<u64>,
    to: Option<u64>,
    length: u64,
) -> Result<(u64, u64), &'static str> {
    let (start, mut end) = match (from, to) {
        (Some(x), Some(z)) => (x, z),                // FromToAll
        (Some(x), None) => (x, length - 1),          // FromTo
        (None, Some(z)) => (length - z, length - 1), // FromEnd
        (None, None) => return Err("You need at least one value to satisfy a range request"),
    };

    if end < start {
        return Err("A byte-range-spec is invalid if the last-byte-pos value is present and less than the first-byte-pos.");
    }
    if end > length {
        end = length
    }

    Ok((start, end))
}

fn range_header_parts(header: &range_header::ByteRange) -> (Option<u64>, Option<u64>) {
    use range_header::ByteRange::{FromTo, FromToAll, Last};
    match *header {
        FromTo(x) => (Some(x), None),
        FromToAll(x, y) => (Some(x), Some(y)),
        Last(x) => (None, Some(x)),
    }
}

impl<'a> Responder<'a, 'a> for SeekStream {
    fn respond_to(self, req: &rocket::Request) -> response::Result<'a> {
        use rocket::http::Status;

        const SERVER_ERROR: Status = Status::InternalServerError;
        const RANGE_ERROR: Status = Status::RangeNotSatisfiable;

        // Get the total length of the stream if not already specified
        let stream_len = match self.length {
            Some(x) => x,
            _ => block_on(
                self
                    .stream
                    .borrow_mut()
                    .seek(SeekFrom::End(0))
                )
                .map_err(|_| SERVER_ERROR)?,
        };

        // Get the mime type, either by inferring it from the stream
        // Or the optional value set in the struct
        let mime_type = match self.content_type {
            Some(x) => String::from(x),
            None => {
                // Infer the mime type of the stream by taking at most a 256 byte sample from the beginning
                // And passing it to the infer_mime_type function
                let mut prelude: [u8; 256] = [0; 256];

                block_on(
                    self
                        .stream
                        .borrow_mut()
                        .rewind()
                )
                    .map_err(|_| SERVER_ERROR)?;

                let c = block_on(self
                    .stream
                    .borrow_mut()
                    .read(&mut prelude)
                )
                    .map_err(|_| SERVER_ERROR)?;

                // Seek to the beginning of the stream to reset the data we took for the sample
                block_on(
                    self.stream
                        .borrow_mut()
                        .seek(std::io::SeekFrom::Start(0))
                    )
                    .map_err(|_| SERVER_ERROR)?;

                infer_mime_type(&prelude[..c])
            }
        };

        // Set the response headers
        let mut resp = Response::new();
        resp.set_raw_header("Accept-Ranges", "bytes");
        resp.set_raw_header("Content-Type", mime_type.clone());

        // If the range header exists, set the response status code to
        // 206 partial content and seek the stream to the requested position
        if let Some(x) = req.headers().get_one("Range") {
            let (ranges, errors) = range_header::ByteRange::parse(x)
                .iter()
                .map(|x| range_header_parts(&x))
                .map(|(start, end)| to_satisfiable_range(start, end, stream_len))
                .partition::<Vec<_>, _>(|x| x.is_ok());

            // If any of the ranges produce an incorrect value,
            // Or the list of ranges is empty.
            // Return a range error.
            if errors.len() > 0 || ranges.len() == 0 {
                for e in errors {
                    println!("{:?}", e.unwrap_err());
                }
                return Err(RANGE_ERROR);
            }

            // Unwrap all the results
            let mut ranges: Vec<(u64, u64)> = ranges.iter().map(|x| x.unwrap()).collect();

            // de-duplicate the list of ranges
            ranges.sort();
            ranges.dedup_by(|&mut (a, b), &mut (c, d)| a == c && b == d);

            // Stream multipart/bytes if multiple ranges have been requested
            if ranges.len() > 1 {
                /*let rd = MultipartReader::new(
                    self.stream.into_inner(),
                    stream_len,
                    mime_type.clone(),
                    ranges,
                );

                resp.set_raw_header(
                    "Content-Type",
                    format!("multipart/byterange; boundary={}", rd.boundary.clone()),
                );
                resp.set_streamed_body(rd);*/
                unimplemented!("currently not supported")
            } else {
                // Stream a single range request if only one was present in the byte ranges
                let &(start, end) = ranges.get(0).unwrap();

                // Seek the stream to the desired position
                block_on(
                    self.stream
                        .borrow_mut()
                        .seek(SeekFrom::Start(start))
                )
                    .map_err(|_| SERVER_ERROR)?;

                let mut stream: Box<dyn AsyncRead + Unpin + Send> = Box::new(self.stream.into_inner());
                if end + 1 < stream_len {
                    stream = Box::new(stream.take(end + 1 - start));
                }

                resp.set_raw_header(
                    "Content-Range",
                    format!("bytes {}-{}/{}", start, end, stream_len),
                );

                // Set the content length to be the length of the partial stream
                resp.set_raw_header("Content-Length", format!("{}", end + 1 - start));
                resp.set_status(rocket::http::Status::PartialContent);

                resp.set_streamed_body(stream)
            }
        } else {
            // No range request; Response with the entire stream
            resp.set_raw_header("Content-Length", format!("{}", stream_len));
            resp.set_streamed_body(self.stream.into_inner());
        }

        Ok(resp)
    }
}
