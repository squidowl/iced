//! Access the clipboard.
use crate::core::clipboard::{ClipboardKind, Content, Error, Kind};
use crate::futures::futures::channel::oneshot;
use crate::runtime::clipboard::Action;

use std::io;
use std::sync::Arc;

use url::Url;
use winit::data_transfer::{
    DataTransferSend, DataTransferSendBuilder, SendData, TypeHint, TypedData,
};
use winit::error::TransferError;
use winit::event_loop::{ActiveEventLoop, AsyncRequestSerial};

type ReadChannel = oneshot::Sender<Result<Content, Error>>;

/// A clipboard that can be used to read and write content.
#[derive(Default)]
pub struct Clipboard {
    pending: Vec<PendingRead>,
}

struct PendingRead {
    serial: AsyncRequestSerial,
    kind: Kind,
    channel: ReadChannel,
}

impl Clipboard {
    /// Creates a new clipboard.
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn run(&mut self, event_loop: &dyn ActiveEventLoop, action: Action) {
        match action {
            Action::Read {
                clipboard_kind,
                kind,
                channel,
            } => match self.read(event_loop, clipboard_kind, kind) {
                Ok(serial) => self.pending.push(PendingRead {
                    serial,
                    kind,
                    channel,
                }),
                Err(error) => {
                    let _ = channel.send(Err(error));
                }
            },
            Action::Write {
                clipboard_kind,
                content,
                channel,
            } => {
                let _ = channel.send(self.write(event_loop, clipboard_kind, content));
            }
        }
    }

    /// Requests content from the given clipboard.
    pub fn read(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        clipboard: ClipboardKind,
        kind: Kind,
    ) -> Result<AsyncRequestSerial, Error> {
        let id = match clipboard {
            ClipboardKind::Standard => event_loop.clipboard(),
            ClipboardKind::Primary => event_loop
                .primary_selection_ext()
                .ok_or(Error::ClipboardUnavailable)?
                .primary_selection(),
        }
        .map_err(clipboard_error)?
        .ok_or(Error::ContentNotAvailable)?;

        let hint = match kind {
            Kind::Text => TypeHint::Plaintext,
            Kind::Html => TypeHint::Html,
            Kind::Files => TypeHint::UriList,
            #[cfg(feature = "image")]
            Kind::Image => png_hint(),
            _ => return Err(Error::ContentNotAvailable),
        };

        let transfer = event_loop.data_transfer(id).map_err(clipboard_error)?;

        if !transfer.has_type(&hint) {
            return Err(Error::ContentNotAvailable);
        }

        event_loop
            .fetch_data_transfer(id, &hint)
            .map_err(clipboard_error)
    }

    /// Writes content to the given clipboard.
    pub fn write(
        &self,
        event_loop: &dyn ActiveEventLoop,
        clipboard: ClipboardKind,
        content: Content,
    ) -> Result<(), Error> {
        let data = encode(content)?;

        match clipboard {
            ClipboardKind::Standard => event_loop.set_clipboard(data),
            ClipboardKind::Primary => event_loop
                .primary_selection_ext()
                .ok_or(Error::ClipboardUnavailable)?
                .set_primary_selection(data),
        }
        .map_err(clipboard_error)
    }

    /// Returns true if `serial` belongs to a pending clipboard read.
    pub(crate) fn receive(&mut self, serial: AsyncRequestSerial, data: &dyn TypedData) -> bool {
        let Some(index) = self.pending.iter().position(|read| read.serial == serial) else {
            return false;
        };

        let result = match decode(self.pending[index].kind, data) {
            Ok(content) => Ok(content),
            Err(error) => Err(match error.kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::Deadlock => return true,
                io::ErrorKind::Unsupported => Error::ContentNotAvailable,
                io::ErrorKind::InvalidData => Error::ConversionFailure,
                _ => Error::Unknown {
                    description: Arc::new(error.to_string()),
                },
            }),
        };

        let read = self.pending.swap_remove(index);
        let _ = read.channel.send(result);

        true
    }
}

fn decode(kind: Kind, data: &dyn TypedData) -> io::Result<Content> {
    match kind {
        Kind::Text => Ok(Content::Text(data.try_as_string()?)),
        Kind::Html => Ok(Content::Html(data.try_as_string()?)),
        Kind::Files => {
            let files: Vec<_> = data
                .try_as_uris()?
                .into_iter()
                .filter_map(|uri| Url::parse(&uri).ok())
                .filter(|uri| uri.scheme() == "file")
                .filter_map(|uri| uri.to_file_path().ok())
                .collect();

            if files.is_empty() {
                return Err(io::ErrorKind::Unsupported.into());
            }

            Ok(Content::Files(files))
        }
        #[cfg(feature = "image")]
        Kind::Image => {
            let image = image::load_from_memory(&data.try_as_bytes()?)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
                .into_rgba8();
            let size = crate::core::Size::new(image.width(), image.height());

            Ok(Content::Image(crate::core::clipboard::Image {
                rgba: crate::core::Bytes::from_owner(image.into_raw()),
                size,
            }))
        }
        _ => Err(io::ErrorKind::Unsupported.into()),
    }
}

fn encode(content: Content) -> Result<Box<dyn DataTransferSend>, Error> {
    let (hint, data) = match content {
        Content::Text(text) => (TypeHint::Plaintext, SendData::String(text)),
        Content::Html(html) => (TypeHint::Html, SendData::String(html)),
        Content::Files(files) => {
            let uris = files
                .into_iter()
                .map(|file| {
                    Url::from_file_path(file)
                        .map(String::from)
                        .map_err(|_| Error::ConversionFailure)
                })
                .collect::<Result<Vec<_>, _>>()?;

            (TypeHint::UriList, SendData::Uris(uris))
        }
        #[cfg(feature = "image")]
        Content::Image(image) => (png_hint(), SendData::Bytes(encode_png(&image)?)),
        _ => return Err(Error::ContentNotAvailable),
    };

    Ok(DataTransferSendBuilder::new(data)
        .with_type(hint, |data, _| Some(data.clone()))
        .build())
}

#[cfg(feature = "image")]
fn png_hint() -> TypeHint {
    TypeHint::Image {
        extension_hint: Some("png"),
    }
}

#[cfg(feature = "image")]
fn encode_png(image: &crate::core::clipboard::Image) -> Result<Vec<u8>, Error> {
    use image::ImageEncoder;

    let (width, height) = (image.size.width, image.size.height);

    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&image.rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|_| Error::ConversionFailure)?;

    Ok(bytes)
}

fn clipboard_error(error: TransferError) -> Error {
    match error {
        TransferError::NotSupported(_) => Error::ClipboardUnavailable,
        TransferError::UnknownTransfer(_) => Error::ContentNotAvailable,
        error => Error::Unknown {
            description: Arc::new(error.to_string()),
        },
    }
}
