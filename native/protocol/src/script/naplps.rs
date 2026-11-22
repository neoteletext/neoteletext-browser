//! The handles of a NAPLPS page's things, which the script host and the
//! decoder must agree on (the script API's specification sections 4.4
//! and 10.1 rule 4; work packages N1 and N2, DEC-7364 item 15 and
//! DEC-7377 item 5): the decoder names the thing under the pointer, and
//! the thing whose painting it reports, by the script host's handle for
//! it, and it can only do so because the handle is made from what the
//! decoder itself knows: a kind, and an object's number in the engine's
//! `NAPLPS_OBJECTS` record.

/// The bits of a handle below its kind.
pub const HANDLE_KIND_SHIFT: u32 = 24;
/// The kinds of NAPLPS things: the backdrop, a symbol, a DRCS character,
/// a texture, and an actor or a region of the cast.
pub const KIND_BACKDROP: i32 = 1;
/// See [`KIND_BACKDROP`].
pub const KIND_SYMBOL: i32 = 2;
/// See [`KIND_BACKDROP`].
pub const KIND_DRCS: i32 = 3;
/// See [`KIND_BACKDROP`].
pub const KIND_TEXTURE: i32 = 4;
/// See [`KIND_BACKDROP`].
pub const KIND_ACTOR: i32 = 5;
/// The backdrop's handle: what `painted` is listened for on, watched by
/// and reported with.
pub const BACKDROP_HANDLE: i32 = KIND_BACKDROP << HANDLE_KIND_SHIFT;

/// The handle of the actor or region that is object `number` of the
/// page's cast.
#[must_use]
pub const fn object_handle(number: u16) -> i32 {
    (KIND_ACTOR << HANDLE_KIND_SHIFT) | number as i32
}

/// The object's number a handle names, when it names an actor or a
/// region.
#[must_use]
pub const fn object_number(handle: i32) -> Option<u16> {
    let rest = handle & ((1 << HANDLE_KIND_SHIFT) - 1);
    if handle >> HANDLE_KIND_SHIFT == KIND_ACTOR && rest > 0 && rest <= u16::MAX as i32 {
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "held to 1 to 65,535 by the line above")]
        let number = rest as u16;
        Some(number)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A handle names its object and nothing else does.
    #[test]
    fn an_objects_handle_names_its_number() {
        assert_eq!(object_handle(3), 0x0500_0003);
        for number in [1, 2, 64, u16::MAX] {
            assert_eq!(object_number(object_handle(number)), Some(number));
        }
        for handle in [0, 3, -1, BACKDROP_HANDLE, KIND_ACTOR << HANDLE_KIND_SHIFT, (KIND_SYMBOL << HANDLE_KIND_SHIFT) | 3, i32::MIN, i32::MAX] {
            assert_eq!(object_number(handle), None, "{handle:#x}");
        }
    }
}
