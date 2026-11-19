//! Version-aware geometry for the Button Area and its visible face.

use ag_iso_stack::object_pool::{
    object::{Button, Object},
    vt_version::VtVersion,
};

/// Inset between a Button's outer rectangle and its face in the editor preview.
///
/// The preview uses a centered four-pixel inset for the bordered face. VT4
/// `No border` uses the whole Button Area; `Suppress border` still reserves the
/// border space. Both options are ignored by earlier VT versions.
pub(crate) fn face_inset(button: &Button, version: VtVersion) -> f32 {
    if version >= VtVersion::Version4 && button.options.no_border {
        0.0
    } else {
        4.0
    }
}

pub(crate) fn object_face_inset(object: &Object, version: VtVersion) -> f32 {
    match object {
        Object::Button(button) => face_inset(button, version),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button() -> Button {
        let Object::Button(button) =
            crate::default_object(ag_iso_stack::object_pool::ObjectType::Button)
        else {
            unreachable!()
        };
        button
    }

    #[test]
    fn face_inset_respects_target_version_and_border_options() {
        let mut button = button();
        assert_eq!(face_inset(&button, VtVersion::Version3), 4.0);
        assert_eq!(face_inset(&button, VtVersion::Version4), 4.0);

        button.options.no_border = true;
        assert_eq!(face_inset(&button, VtVersion::Version3), 4.0);
        assert_eq!(face_inset(&button, VtVersion::Version4), 0.0);

        button.options.suppress_border = true;
        assert_eq!(face_inset(&button, VtVersion::Version4), 0.0);
        button.options.no_border = false;
        assert_eq!(face_inset(&button, VtVersion::Version4), 4.0);
    }
}
