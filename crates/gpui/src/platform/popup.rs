use bitflags::bitflags;
use thiserror::Error;

use crate::{Bounds, Pixels, Point, Size};

/// Options for a parent-anchored popup window such as a menu, dropdown, context menu or tooltip.
///
/// A popup is placed relative to its parent window rather than at an absolute screen position. The
/// application describes where the popup should sit relative to an anchor rectangle on the parent,
/// and the platform turns that description into a real position. This lets a system that owns
/// window placement honor the request, and lets a platform with absolute coordinates resolve it
/// itself.
///
/// All coordinates and sizes are in logical pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PopupOptions {
    /// The rectangle on the parent window that the popup is positioned relative to, measured from
    /// the top-left of the parent window's content area.
    ///
    /// For example, a dropdown menu uses the bounds of the button that opened it.
    pub anchor_rect: Bounds<Pixels>,

    /// The size of the popup surface.
    pub size: Size<Pixels>,

    /// Which point of [`Self::anchor_rect`] the popup is anchored to.
    pub anchor: PopupAnchor,

    /// The direction in which the popup extends away from the anchor point. A dropdown that drops
    /// below its button anchors to [`PopupAnchor::BottomLeft`] with a gravity of
    /// [`PopupGravity::BottomRight`] so it grows down and to the right.
    pub gravity: PopupGravity,

    /// How the popup may be adjusted by the platform if the requested placement would put it
    /// off-screen.
    pub constraint_adjustment: PopupConstraintAdjustment,

    /// An additional offset applied to the popup after anchoring.
    pub offset: Point<Pixels>,

    /// Whether the popup should take an explicit input grab.
    ///
    /// Grabbing popups behave like menus. They take keyboard focus and are dismissed when the user
    /// clicks outside of them or presses a dismissing key. Use it for menus and comboboxes, not for
    /// tooltips or other passive popups.
    ///
    /// A grab has to be requested while the triggering input is still active, in practice the press
    /// of the mouse button that opens the popup. Open grabbing popups from a mouse-down handler
    /// rather than a click handler, otherwise the button is already released and the grab is
    /// refused.
    ///
    /// Automatic dismissal only covers input aimed at other applications. A click elsewhere in your
    /// own application still reaches it as usual and does not dismiss the popup, so handling that
    /// case is up to you.
    pub grab: bool,
}

// `PopupOptions` holds a `Size<Pixels>`, which is only `PartialEq` because `Size` may be generic
// over floating-point types. `Pixels` itself is `Eq`, so equality is total here and implementing
// `Eq` by hand is sound. This keeps `WindowKind` able to derive `Eq`.
impl Eq for PopupOptions {}

/// The point of the anchor rectangle that a popup is anchored to.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum PopupAnchor {
    /// Anchor to the center of the anchor rectangle.
    #[default]
    Center,
    /// Anchor to the center of the top edge.
    Top,
    /// Anchor to the center of the bottom edge.
    Bottom,
    /// Anchor to the center of the left edge.
    Left,
    /// Anchor to the center of the right edge.
    Right,
    /// Anchor to the top-left corner.
    TopLeft,
    /// Anchor to the bottom-left corner.
    BottomLeft,
    /// Anchor to the top-right corner.
    TopRight,
    /// Anchor to the bottom-right corner.
    BottomRight,
}

/// The direction in which a popup extends away from its anchor point.
///
/// For instance, a gravity of [`PopupGravity::BottomRight`] places the popup below and to the right
/// of the anchor point.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum PopupGravity {
    /// The popup is centered over the anchor point.
    #[default]
    Center,
    /// The popup extends upwards from the anchor point.
    Top,
    /// The popup extends downwards from the anchor point.
    Bottom,
    /// The popup extends to the left of the anchor point.
    Left,
    /// The popup extends to the right of the anchor point.
    Right,
    /// The popup extends up and to the left of the anchor point.
    TopLeft,
    /// The popup extends down and to the left of the anchor point.
    BottomLeft,
    /// The popup extends up and to the right of the anchor point.
    TopRight,
    /// The popup extends down and to the right of the anchor point.
    BottomRight,
}

bitflags! {
    /// Describes how a popup may be adjusted by the platform if the requested placement would put
    /// it off-screen. If no flags are set, the popup is placed exactly as requested and may be
    /// clipped.
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
    pub struct PopupConstraintAdjustment: u32 {
        /// The popup may be slid horizontally to stay on-screen.
        const SLIDE_X = 1;
        /// The popup may be slid vertically to stay on-screen.
        const SLIDE_Y = 2;
        /// The popup's anchor and gravity may be flipped horizontally to stay on-screen.
        const FLIP_X = 4;
        /// The popup's anchor and gravity may be flipped vertically to stay on-screen.
        const FLIP_Y = 8;
        /// The popup may be shrunk horizontally to stay on-screen.
        const RESIZE_X = 16;
        /// The popup may be shrunk vertically to stay on-screen.
        const RESIZE_Y = 32;
    }
}

/// Returned when the current platform has no native popup implementation yet.
///
/// Native popups are separate from gpui's in-window popovers, which are drawn as elements inside an
/// existing window. A caller that wants a popup on every platform should treat this error as a cue
/// to fall back to that in-window rendering.
#[derive(Debug, Error)]
#[error("popups are not supported on this platform")]
pub struct PopupNotSupportedError;
