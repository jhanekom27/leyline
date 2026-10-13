//! Ticket parsing, topic derivation, and clipboard payload helpers.

use super::*;

/// message encryption key).
pub(super) fn topic_for_secret(secret: &RoomSecret) -> TopicId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"leyline-room:");
    hasher.update(secret.as_bytes());
    hasher.finalize().into()
}

/// A decoded ticket's payload, as returned by `parse_join_arg`: its room
/// secret, the sharer's address, and whether it's DM-flagged (see
/// `ticket::ChannelTicket::dm`).
type ParsedTicket = (RoomSecret, EndpointAddr, bool);

/// Interprets a `/join` argument as either a bare channel name or an
/// invite ticket.
///
/// If `arg` doesn't even have the ticket prefix, it's treated as a plain
/// channel name with no room secret yet -- see `Net::join`, which
/// generates a fresh one for a name it's never seen before. If it *does*
/// have the prefix but still fails to decode -- e.g. because it was
/// truncated when copied out of a narrow terminal -- that's reported as an
/// error instead of silently joining a bogus channel named after the
/// mangled string. The ticket's `dm` marker rides along too, so `Net::join`
/// can tell a DM invite apart from an ordinary one (see
/// `NetEvent::DmJoined`).
pub(super) fn parse_join_arg(arg: &str) -> Result<(String, Option<ParsedTicket>), String> {
    match ChannelTicket::decode_string(arg) {
        Ok(ticket) => Ok((ticket.name, Some((ticket.secret, ticket.addr, ticket.dm)))),
        Err(ParseError::Kind { .. }) => Ok((arg.trim().to_string(), None)),
        Err(err) => Err(format!(
            "that looks like an invite ticket but it won't decode (maybe it got truncated when copied?): {err}"
        )),
    }
}

/// `true` if `addr` names our own endpoint id.
///
/// Used to detect a self-referential invite ticket -- e.g. pasting your
/// own `/invite` ticket back in after a restart, since a ticket always
/// encodes the *sharer's* address (`Net::ticket_for`), not a durable
/// channel meeting point. iroh treats connecting to yourself as a no-op,
/// so keeping such an address as a bootstrap candidate would otherwise
/// produce a silently isolated topic subscription instead of a clear
/// outcome.
pub(super) fn is_self(addr: &EndpointAddr, our_id: [u8; 32]) -> bool {
    *addr.id.as_bytes() == our_id
}

/// Encodes clipboard pixel data (`ImageData`'s raw RGBA8 buffer, row-major,
/// no padding -- see `Net::paste`) as a PNG, the `/paste` counterpart to
/// `files::expand_path`'s filesystem handling: clipboard image data has no
/// filename or original file format to preserve, so PNG (lossless,
/// universally viewable) is what actually gets shared, not the raw pixel
/// buffer itself. Always produces a single static frame -- `ImageData` has
/// no concept of multiple frames, so an animated image copied as rendered
/// pixels (as opposed to a file -- see `Net::paste`'s file-list check) can
/// never round-trip as a GIF through this path.
pub(super) fn encode_png(image: ImageData) -> anyhow::Result<Vec<u8>> {
    let width = u32::try_from(image.width).context("clipboard image width overflowed u32")?;
    let height = u32::try_from(image.height).context("clipboard image height overflowed u32")?;
    let buffer = RgbaImage::from_raw(width, height, image.bytes.into_owned())
        .context("clipboard image dimensions didn't match its pixel buffer")?;
    let mut png_bytes = Vec::new();
    buffer
        .write_to(&mut std::io::Cursor::new(&mut png_bytes), ImageFormat::Png)
        .context("failed to encode clipboard image as PNG")?;
    Ok(png_bytes)
}
