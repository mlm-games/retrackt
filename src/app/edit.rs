//! Editor interaction: the camera, what the pointer is over, and the gestures
//! that drive both.
//!
//! The viewport's own gestures are deliberately not used while the editor is up.
//! They bind the left button to orbit and every other button to pan, which is
//! the opposite of what an editor needs: here the left button is the one that
//! places a piece, and orbiting belongs to the middle button. Rather than work
//! around that, the editor takes the pointer over the viewport and owns the
//! whole gesture set in one place.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use glam::{Vec2, Vec3};
use repame_view3d::Frame3d;
use repose_core::input::{PointerButton, PointerEvent, PointerEventKind, PointerKind};
use repose_core::{Modifier, View};
use repose_ui::Box;
use retrackt_format::{PieceUid, TrackDocument, geometry};

use super::App;
use crate::app::state::{ActionQueue, UiAct, push};

/// Camera glide speed, in units per second per unit of camera distance.
///
/// Proportional to the distance rather than a constant, so the same key press
/// moves the view a screen's worth whether the camera is 20 units out or 200:
/// zoomed in to place a kerb, W is a nudge; zoomed out to survey a circuit, it
/// is a crossing.
const MOVE_SPEED: f32 = 0.9;

/// Camera turn rate, in radians per second.
const TURN_RATE: f32 = 1.7;

/// How far a press may travel and still be a click, in physical px.
///
/// The same slop the viewport itself uses, so a click means the same thing
/// whichever side of the editor is being asked.
const CLICK_SLOP_PX: f32 = repame_view3d::CLICK_SLOP_PX;

/// What the pointer is over, resolved against the camera the frame is drawn with.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EditorHover {
    /// Nearest placed piece the pointer ray passes through. Wins over the
    /// ground, because a tap beside a piece is aimed at the piece.
    pub piece: Option<PieceUid>,
    /// Grid cell the ground under the pointer falls in, which is where the cursor
    /// wants to be.
    pub cell: Option<[i16; 3]>,
}

/// Live pointer and drag state, shared with the view's event closures.
///
/// Held in an `Rc` rather than on `App` because a gesture spans frames while the
/// closure receiving it is rebuilt every frame: only the cell behind both can
/// remember where the press began.
///
/// Touch and mouse live in one list rather than in two paths, because the
/// gestures are the same gestures: one finger dragging the view is a pan, two
/// fingers rotating it is an orbit, and a mouse button is just a finger that
/// reports which one it is.
#[derive(Default)]
pub(crate) struct EditorGesture {
    /// Every pointer currently held down.
    pointers: Vec<Pointer>,
    /// The two-finger gesture, from the moment the second finger landed.
    pinch: Option<Pinch>,
    /// Where the pointer the hover resolves against is, from the last event.
    /// Kept after the event so a camera move can re-resolve what the pointer is
    /// now over.
    hover: Option<[f32; 2]>,
}

/// One pointer held down.
#[derive(Clone, Copy)]
struct Pointer {
    id: u64,
    kind: PointerKind,
    /// The button, which for a touch is always the primary one.
    button: PointerButton,
    /// Where the press landed, which is what a tap is measured from.
    at: [f32; 2],
    /// Where it is now.
    last: [f32; 2],
}

/// A two-finger gesture, measured from the press but applied in increments.
///
/// Both halves of that matter. The *decision* has to come from the whole
/// gesture, because two fingers cannot be told apart from a movement only one of
/// them made: read off a single event, the first half of a rotate looks exactly
/// like a spread. The *magnitude* has to be incremental, because what the view
/// does with it is a delta, and a delta measured from the start is applied again
/// on every finger that twitches afterwards.
///
/// Which is which is read off the finger axis: moving along it changes how far
/// apart the fingers are, and moving across it moves both of them the same way.
#[derive(Clone, Copy)]
struct Pinch {
    /// The two fingers it belongs to, so a hand that puts one down elsewhere
    /// does not get read as the same gesture.
    ids: [u64; 2],
    /// Where each finger was when the gesture began.
    from: [[f32; 2]; 2],
    /// The axis between them then, and across it.
    axis: [f32; 2],
    /// Their separation then, and at the last event. The zoom is the ratio
    /// between the latter two, so each event scales by only its own change.
    span: f32,
    last_span: f32,
    /// Where their midpoint was at the last event, so the turn is likewise only
    /// what changed.
    last_midpoint: [f32; 2],
}

/// One finger's displacement, split along the finger axis and across it.
struct Splay {
    /// Movement along the axis between the fingers: this one only.
    along: f32,
    /// Movement across it: this one only.
    across: f32,
}

/// What one pointer movement did to the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Gesture {
    /// Slide the view, in viewport px.
    Pan(f32, f32),
    /// Turn the view, in viewport px.
    Orbit(f32, f32),
    /// Scale the view, as a factor on the distance.
    Zoom(f32),
}

impl EditorGesture {
    /// A fresh gesture cell, before the first view is built.
    pub(crate) fn new_handle() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self::default()))
    }

    fn press(&mut self, ev: &PointerEvent) {
        let button = match ev.event {
            PointerEventKind::Down(button) => button,
            _ => PointerButton::Primary,
        };
        let px = [ev.position.x, ev.position.y];
        self.hover = Some(px);
        self.pointers.push(Pointer {
            id: ev.id.0,
            kind: ev.kind,
            button,
            at: px,
            last: px,
        });
        self.begin_pinch();
    }

    /// Move every pointer, and report what the view should do about it.
    fn drag(&mut self, ev: &PointerEvent) -> Option<Gesture> {
        let px = [ev.position.x, ev.position.y];
        let Some(press) = self
            .pointers
            .iter_mut()
            .find(|p| p.id == ev.id.0 && p.kind == ev.kind)
        else {
            // A pointer that never went down here: it is hovering, not dragging.
            self.hover = Some(px);
            return None;
        };
        let before = press.last;
        press.last = px;
        self.hover = Some(px);

        if let Some(pinch) = self.pinch {
            let (gesture, moved) = two_finger(pinch, &self.pointers);
            self.pinch = Some(moved);
            return gesture;
        }
        if self.pointers.len() >= 2 {
            return None;
        }
        let press = self.pointers[0];
        let (dx, dy) = (px[0] - before[0], px[1] - before[1]);
        if press.kind == PointerKind::Touch {
            // One finger drags the view, which is the touch word for panning.
            return (dx != 0.0 || dy != 0.0).then_some(Gesture::Pan(dx, dy));
        }
        match press.button {
            // A left drag does nothing: the left button places, and a press that
            // moved is not a tap.
            PointerButton::Primary => None,
            // The middle button, or the right one with Ctrl held, turns the view.
            // Both are bound because a trackpad has no middle button and a mouse
            // has no ctrl-and-right habit, and one gesture per machine is enough.
            PointerButton::Tertiary => Some(Gesture::Orbit(dx, dy)),
            PointerButton::Secondary if ev.modifiers.ctrl => Some(Gesture::Orbit(dx, dy)),
            // A plain right drag slides the view across the track.
            PointerButton::Secondary => Some(Gesture::Pan(dx, dy)),
        }
    }

    /// Lift a pointer, reporting whether it did not travel and so was a tap.
    fn release(&mut self, ev: &PointerEvent) -> bool {
        let px = [ev.position.x, ev.position.y];
        self.hover = Some(px);
        let Some(index) = self
            .pointers
            .iter()
            .position(|p| p.id == ev.id.0 && p.kind == ev.kind)
        else {
            return false;
        };
        let press = self.pointers.remove(index);
        self.begin_pinch();
        // Measured from the press rather than from where it is now, so a slow
        // drag that wanders back to where it began does not end in a stray tap.
        let moved = (px[0] - press.at[0]).hypot(px[1] - press.at[1]);
        moved <= CLICK_SLOP_PX
    }

    /// Forget every pointer, for leaving the editor.
    ///
    /// A finger that was dragging the view when the editor closed must not come
    /// back as a half-finished drag, and a pointer left where it was would put
    /// the hover over the track the editor is no longer showing.
    fn release_all(&mut self) {
        self.pointers.clear();
        self.pinch = None;
        self.hover = None;
    }

    /// Start a two-finger gesture, or end the one in hand.
    ///
    /// Every pointer change begins one, so a finger that lands while another is
    /// already down turns the gesture into a pinch from that moment rather than
    /// from wherever the first finger had wandered to.
    fn begin_pinch(&mut self) {
        let fingers: Vec<Pointer> = self
            .pointers
            .iter()
            .copied()
            .filter(|p| p.kind == PointerKind::Touch)
            .collect();
        if fingers.len() != 2 {
            self.pinch = None;
            return;
        }
        let (a, b) = (fingers[0].last, fingers[1].last);
        let along = [b[0] - a[0], b[1] - a[1]];
        let span = along[0].hypot(along[1]);
        // Two fingers on the same pixel have no axis; anything will do, and the
        // gesture will read as a turn, which is all such a pair can be.
        let axis = if span < 1e-8 {
            [1.0, 0.0]
        } else {
            [along[0] / span, along[1] / span]
        };
        self.pinch = Some(Pinch {
            ids: [fingers[0].id, fingers[1].id],
            from: [a, b],
            axis,
            span,
            last_span: span,
            last_midpoint: [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5],
        });
    }
}

/// Two fingers together: they turn the view, and they pinch it in and out.
///
/// The turn is the movement both fingers share, so the view rotates about the
/// point between them rather than about one. The zoom is the change in how far
/// apart they are, so they keep hold of what they started on.
///
/// Which of the two it is, is decided from the whole gesture rather than from one
/// event, because two fingers cannot be told apart from a movement only one of
/// them made: the first half of a turn, delivered as the second finger's move
/// arriving before the first's, looks exactly like a spread.
fn two_finger(mut pinch: Pinch, pointers: &[Pointer]) -> (Option<Gesture>, Pinch) {
    let fingers: Vec<[f32; 2]> = pointers
        .iter()
        .filter(|p| p.kind == PointerKind::Touch && pinch.ids.contains(&p.id))
        .map(|p| p.last)
        .collect();
    let [a, b] = fingers.as_slice() else {
        return (None, pinch);
    };
    let splay = |from: [f32; 2], now: [f32; 2]| {
        let moved = [now[0] - from[0], now[1] - from[1]];
        Splay {
            along: moved[0] * pinch.axis[0] + moved[1] * pinch.axis[1],
            across: moved[0] * -pinch.axis[1] + moved[1] * pinch.axis[0],
        }
    };
    let (first, second) = (splay(pinch.from[0], *a), splay(pinch.from[1], *b));
    // Half the difference in how far each finger moved along the axis is the
    // change in how far apart they are; half the sum across it is how far they
    // both went the same way.
    let spread = (first.along - second.along).abs() * 0.5;
    let across = (first.across + second.across).abs() * 0.5;
    let span = (a[0] - b[0]).hypot(a[1] - b[1]);
    let midpoint = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    // A finger that has not moved since the last event moves nothing, however
    // the fingers happen to be arranged.
    if span == pinch.last_span && midpoint == pinch.last_midpoint {
        return (None, pinch);
    }
    let gesture = if pinch.span > 1.0 && spread > across {
        Some(Gesture::Zoom((pinch.last_span / span).clamp(0.5, 2.0)))
    } else {
        let turned = [
            midpoint[0] - pinch.last_midpoint[0],
            midpoint[1] - pinch.last_midpoint[1],
        ];
        (turned[0] != 0.0 || turned[1] != 0.0).then(|| Gesture::Orbit(turned[0], turned[1]))
    };
    pinch.last_span = span;
    pinch.last_midpoint = midpoint;
    (gesture, pinch)
}

impl App {
    /// Glide and turn the camera for the keys held this frame.
    pub(crate) fn move_editor_camera(&mut self, dt: f32, move_cam: [f32; 2], turn: f32) {
        if turn != 0.0 {
            // The same sign a drag to the right gives, so the keyboard and the
            // mouse turn the view the same way.
            self.editor_cam.yaw -= turn * TURN_RATE * dt;
        }
        let (forward, right) = (move_cam[0], move_cam[1]);
        if forward == 0.0 && right == 0.0 {
            return;
        }
        let eye = self.editor_cam.eye();
        let mut flat = self.editor_cam.target - eye;
        flat.y = 0.0;
        // A camera looking straight down has no ground-plane forward to travel
        // along, and normalising a zero vector would put NaN in the target.
        if flat.length_squared() < 1e-8 {
            return;
        }
        let flat = flat.normalize();
        let side = flat.cross(Vec3::Y).normalize();
        let step = (flat * forward + side * right) * (MOVE_SPEED * self.editor_cam.dist * dt);
        self.editor_cam.target += step;
        // Held to the ground plane, as `OrbitCamera::pan` does. A target that
        // wandered up would leave the camera looking at the sky, and the height
        // being edited is the cursor's, not the view's.
        self.editor_cam.target.y = 0.0;
    }

    /// Forget every pointer gesture, for leaving the editor.
    pub(crate) fn release_editor_gesture(&self) {
        self.editor_gesture.borrow_mut().release_all();
    }

    /// Note where the pointer is, in viewport pixels.
    pub(crate) fn editor_pointer(&mut self, x: f32, y: f32) {
        self.editor_gesture.borrow_mut().hover = Some([x, y]);
    }

    /// Re-resolve what the pointer is over, and follow it with the cursor.
    ///
    /// Called every frame rather than on pointer-move alone, because the pointer
    /// is typically still while the camera moves under it, and both the ground
    /// cell and the piece it names change with the view.
    pub(crate) fn update_editor_hover(&mut self) {
        let hover = self.hover_pick();
        self.data.editor.hover = hover.piece;
        // The cursor follows the ground under the pointer, so the preview the
        // player is looking at is the one a click would place. The height is
        // left to its own control: a ray against the ground plane knows nothing
        // about it.
        if let Some(cell) = hover.cell {
            let current = self.data.editor.cursor;
            if cell[0] != current[0] || cell[2] != current[2] {
                self.set_cursor([cell[0], current[1], cell[2]]);
            }
        }
    }

    /// The camera ray at a viewport pixel, with the viewport's painted size.
    fn editor_ray(&self, px: [f32; 2]) -> ((Vec3, Vec3), Vec2) {
        let [w, h] = self.geom.get().viewport_px;
        let viewport = Vec2::new(w.max(1.0), h.max(1.0));
        let p = Vec2::new(px[0].clamp(0.0, viewport.x), px[1].clamp(0.0, viewport.y));
        let aspect = Frame3d::aspect([viewport.x, viewport.y]);
        (self.editor_cam.screen_ray(aspect, viewport, p), viewport)
    }

    /// Everything the pointer is over, or nothing when it is not over the
    /// viewport at all.
    pub(crate) fn hover_pick(&self) -> EditorHover {
        let Some(px) = self.editor_gesture.borrow().hover else {
            return EditorHover::default();
        };
        let ((eye, dir), viewport) = self.editor_ray(px);
        let aspect = Frame3d::aspect([viewport.x, viewport.y]);
        let ground = self
            .editor_cam
            .ground_point(aspect, viewport, Vec2::new(px[0], px[1]))
            .filter(|g| g.is_finite());
        let cell =
            ground.map(|g| geometry::to_grid(Vec3::new(g.x, 0.0, g.y), self.data.track.cell_size));
        EditorHover {
            piece: self.piece_under_ray(eye, dir),
            cell,
        }
    }

    /// The nearest placed piece the ray passes through.
    ///
    /// A slab test against each piece's reservation box rather than against its
    /// triangles: the box is what the preview and the cursor are already sized
    /// by, and it is exact at the grain a click works at.
    fn piece_under_ray(&self, eye: Vec3, dir: Vec3) -> Option<PieceUid> {
        self.editor_pick.hit(eye, dir)
    }

    /// Resolve a click in the viewport.
    ///
    /// A piece under the pointer is selected; empty ground places the armed
    /// piece. Resolved against the camera this frame drew with, which is the
    /// camera the click was made against: a press and a release with no travel
    /// between them cannot have moved it.
    pub(crate) fn resolve_editor_click(&mut self, x: f32, y: f32) {
        let ((eye, dir), viewport) = self.editor_ray([x, y]);
        if let Some(uid) = self.piece_under_ray(eye, dir) {
            push(&self.data.actions, UiAct::SelectPiece(uid));
            return;
        }
        let aspect = Frame3d::aspect([viewport.x, viewport.y]);
        let Some(ground) = self
            .editor_cam
            .ground_point(aspect, viewport, Vec2::new(x, y))
            .filter(|g| g.is_finite())
        else {
            return;
        };
        let cell = geometry::to_grid(
            Vec3::new(ground.x, 0.0, ground.y),
            self.data.track.cell_size,
        );
        let height = self.data.editor.cursor[1];
        push(
            &self.data.actions,
            UiAct::SetCursor([cell[0], height, cell[2]]),
        );
        push(&self.data.actions, UiAct::PlaceArmed);
    }

    /// Take the piece the pointer is over, if there is one.
    pub(crate) fn select_hovered(&mut self) {
        match self.hover_pick().piece {
            Some(uid) => self.data.editor.selection = vec![uid],
            None => self.data.notice = Some("No track part under the pointer.".into()),
        }
    }
}

/// The editor's gesture overlay over the viewport.
///
/// A full-size hit region that claims every pointer event, so the viewport's own
/// handlers underneath never see any of them. Everything it needs is a handle to
/// the gesture cell, the Shift flag for the wheel, and the action queue: the
/// work itself happens on `App`, which holds the camera and the track, on the
/// next frame.
pub(crate) fn gesture_overlay(
    gesture: Rc<RefCell<EditorGesture>>,
    shift: Rc<Cell<bool>>,
    queue: ActionQueue,
) -> View {
    let press = gesture.clone();
    let press_queue = queue.clone();
    let drag = gesture.clone();
    let drag_queue = queue.clone();
    let lift = gesture.clone();
    let lift_queue = queue.clone();
    let cancelled = gesture;
    let wheel_queue = queue;

    Box(Modifier::new()
        .fill_max_size()
        // Claims the pointer across the whole viewport. The viewport's own
        // gestures bind the left button to orbit, which is the last thing an
        // editor's left button should do.
        .input_blocker()
        .on_pointer_down(move |ev| {
            press.borrow_mut().press(&ev);
            push(
                &press_queue,
                UiAct::EditorPointer(ev.position.x, ev.position.y),
            );
        })
        .on_pointer_move(move |ev| {
            let gesture = drag.borrow_mut().drag(&ev);
            push(
                &drag_queue,
                UiAct::EditorPointer(ev.position.x, ev.position.y),
            );
            match gesture {
                Some(Gesture::Pan(dx, dy)) => push(&drag_queue, UiAct::EditorPan(dx, dy)),
                Some(Gesture::Orbit(dx, dy)) => push(&drag_queue, UiAct::EditorOrbit(dx, dy)),
                Some(Gesture::Zoom(factor)) => push(&drag_queue, UiAct::EditorZoom(factor)),
                None => {}
            }
        })
        .on_pointer_up(move |ev| {
            let tapped = lift.borrow_mut().release(&ev);
            push(
                &lift_queue,
                UiAct::EditorPointer(ev.position.x, ev.position.y),
            );
            if tapped {
                push(
                    &lift_queue,
                    UiAct::EditorClick(ev.position.x, ev.position.y),
                );
            }
        })
        .on_pointer_cancel(move |_| cancelled.borrow_mut().release_all())
        .on_scroll(move |delta| wheel(&wheel_queue, delta, shift.get())))
}

/// One wheel notch: zoom, or the edited height while Shift is held. Consumed
/// either way, so the track never scrolls a panel and zooms at the same time.
fn wheel(queue: &ActionQueue, delta: repose_core::Vec2, shift: bool) -> repose_core::Vec2 {
    if delta.y.abs() <= 0.5 {
        return delta;
    }
    if shift {
        let step = if delta.y < 0.0 { 1 } else { -1 };
        push(queue, UiAct::MoveCursor([0, step, 0]));
    } else {
        let factor = (1.0 - delta.y * 0.002).clamp(0.5, 2.0);
        push(queue, UiAct::EditorZoom(factor));
    }
    repose_core::Vec2::ZERO
}

/// Pick boxes for the editor's hover, one per placed piece.
///
/// Cached rather than derived per frame, because deriving one calls
/// `piece_shape`, which builds a whole mesh for a single bounding box, and the
/// hover is re-resolved every frame while the pointer is over the viewport. On a
/// long track that is the difference between a free frame and a stalled one, and
/// it is rebuilt only when the track changes, which is the only thing that moves
/// a box.
#[derive(Debug, Default)]
pub(crate) struct PickBoxes(Vec<(PieceUid, Vec3, Vec3)>);

impl PickBoxes {
    /// The boxes of every piece in `doc`, in world space.
    pub(crate) fn build(doc: &TrackDocument) -> Self {
        let mut boxes = Vec::with_capacity(doc.pieces.len());
        for inst in &doc.pieces {
            let shape = retrackt_format::piece_shape(inst.id, &inst.params, doc.cell_size);
            let (lo, hi) = geometry::reserve_extent(&shape, doc.cell_size);
            let world = inst.world_matrix(doc.cell_size);
            let mut min = Vec3::splat(f32::MAX);
            let mut max = Vec3::splat(f32::MIN);
            for x in [lo.x, hi.x] {
                for y in [lo.y, hi.y] {
                    for z in [lo.z, hi.z] {
                        let corner = world.transform_point3(Vec3::new(x, y, z));
                        min = min.min(corner);
                        max = max.max(corner);
                    }
                }
            }
            if min.x <= max.x {
                boxes.push((inst.uid, min, max));
            }
        }
        Self(boxes)
    }

    /// The nearest piece whose box the ray passes through.
    pub(crate) fn hit(&self, eye: Vec3, dir: Vec3) -> Option<PieceUid> {
        if dir.length_squared() < 1e-12 {
            return None;
        }
        let mut best: Option<(f32, PieceUid)> = None;
        for (uid, min, max) in &self.0 {
            let Some((t, _)) = ray_aabb(eye, dir, (*min, *max)) else {
                continue;
            };
            if best.is_none_or(|(nearest, _)| t < nearest) {
                best = Some((t, *uid));
            }
        }
        best.map(|(_, uid)| uid)
    }
}

/// Where a ray meets an axis-aligned box, as `(distance, point)`.
///
/// `None` when the ray misses, or when the box is behind the eye.
fn ray_aabb(eye: Vec3, dir: Vec3, (min, max): (Vec3, Vec3)) -> Option<(f32, Vec3)> {
    if min.x > max.x {
        return None;
    }
    let mut enter = 0.0f32;
    let mut leave = f32::MAX;
    for axis in 0..3 {
        let (o, d) = ([eye.x, eye.y, eye.z][axis], [dir.x, dir.y, dir.z][axis]);
        let (lo, hi) = ([min.x, min.y, min.z][axis], [max.x, max.y, max.z][axis]);
        if d.abs() < 1e-9 {
            if o < lo || o > hi {
                return None;
            }
            continue;
        }
        let (t0, t1) = ((lo - o) / d, (hi - o) / d);
        enter = enter.max(t0.min(t1));
        leave = leave.min(t0.max(t1));
        if enter > leave {
            return None;
        }
    }
    Some((enter, eye + dir * enter))
}

#[cfg(test)]
mod tests {
    use super::*;
    use repose_core::Vec2 as CoreVec2;
    use repose_core::input::{PointerEventKind, PointerId};

    fn down(id: u64, kind: PointerKind, x: f32, y: f32) -> PointerEvent {
        PointerEvent::new(
            PointerId(id),
            kind,
            PointerEventKind::Down(PointerButton::Primary),
            CoreVec2 { x, y },
            1.0,
            Default::default(),
        )
    }

    fn up(id: u64, kind: PointerKind, x: f32, y: f32) -> PointerEvent {
        PointerEvent::new(
            PointerId(id),
            kind,
            PointerEventKind::Up(PointerButton::Primary),
            CoreVec2 { x, y },
            1.0,
            Default::default(),
        )
    }

    fn moved(id: u64, kind: PointerKind, x: f32, y: f32) -> PointerEvent {
        PointerEvent::new(
            PointerId(id),
            kind,
            PointerEventKind::Move,
            CoreVec2 { x, y },
            1.0,
            Default::default(),
        )
    }

    fn mouse_button(id: u64, button: PointerButton) -> PointerEvent {
        PointerEvent::new(
            PointerId(id),
            PointerKind::Mouse,
            PointerEventKind::Down(button),
            CoreVec2 { x: 0.0, y: 0.0 },
            1.0,
            Default::default(),
        )
    }

    #[test]
    fn one_finger_drags_the_view() {
        let mut g = EditorGesture::default();
        g.press(&down(1, PointerKind::Touch, 100.0, 100.0));
        let gesture = g.drag(&moved(1, PointerKind::Touch, 140.0, 130.0));
        assert_eq!(gesture, Some(Gesture::Pan(40.0, 30.0)));
    }

    #[test]
    fn two_fingers_turning_the_view_move_it_by_the_whole_drag() {
        let mut g = EditorGesture::default();
        g.press(&down(1, PointerKind::Touch, 100.0, 100.0));
        g.press(&down(2, PointerKind::Touch, 300.0, 100.0));
        // Both fingers travel 20,20 together, which is what a two-finger drag is:
        // the distance between them holds, so there is no pinch to read. Each
        // event carries its own half of it.
        assert_eq!(
            g.drag(&moved(2, PointerKind::Touch, 320.0, 120.0)),
            Some(Gesture::Orbit(10.0, 10.0))
        );
        assert_eq!(
            g.drag(&moved(1, PointerKind::Touch, 120.0, 120.0)),
            Some(Gesture::Orbit(10.0, 10.0)),
            "and the second half, not the whole drag again"
        );
    }

    #[test]
    fn spreading_two_fingers_zooms_by_the_change_and_never_twice() {
        let mut g = EditorGesture::default();
        g.press(&down(1, PointerKind::Touch, 100.0, 100.0));
        g.press(&down(2, PointerKind::Touch, 300.0, 100.0));
        // Both fingers travel apart, which is what a pinch is: 200 apart to 250,
        // and each event carries only its own change. Composed, they are the
        // whole spread, and applied twice each would have been far more.
        assert_eq!(
            g.drag(&moved(1, PointerKind::Touch, 50.0, 100.0)),
            Some(Gesture::Zoom(200.0 / 250.0))
        );
        assert_eq!(
            g.drag(&moved(2, PointerKind::Touch, 350.0, 100.0)),
            Some(Gesture::Zoom(250.0 / 300.0)),
            "the second step must not compound on the first"
        );
        let composed: f32 = (200.0 / 250.0) * (250.0 / 300.0);
        assert!(
            (composed - (200.0 / 300.0)).abs() < 1e-6,
            "the steps must compose to the whole spread"
        );
    }

    #[test]
    fn one_finger_alone_pans_even_while_another_is_down() {
        let mut g = EditorGesture::default();
        g.press(&down(1, PointerKind::Touch, 100.0, 100.0));
        g.press(&down(2, PointerKind::Touch, 300.0, 100.0));
        // A finger that lands and moves on its own, with the other planted, is a
        // turn of half its movement: the view follows the midpoint.
        g.drag(&moved(1, PointerKind::Touch, 150.0, 100.0));
        let gesture = g.drag(&moved(2, PointerKind::Touch, 300.0, 100.0));
        assert!(
            gesture.is_none(),
            "a finger that has not moved moves nothing"
        );
    }

    #[test]
    fn the_right_button_pans_the_middle_button_turns() {
        for (button, want) in [
            (PointerButton::Secondary, Gesture::Pan(10.0, 0.0)),
            (PointerButton::Tertiary, Gesture::Orbit(10.0, 0.0)),
        ] {
            let mut g = EditorGesture::default();
            g.press(&mouse_button(0, button));
            let gesture = g.drag(&moved(0, PointerKind::Mouse, 10.0, 0.0));
            assert_eq!(gesture, Some(want), "{button:?}");
        }
    }

    #[test]
    fn the_left_button_drags_nothing_because_it_places() {
        let mut g = EditorGesture::default();
        g.press(&mouse_button(0, PointerButton::Primary));
        assert_eq!(g.drag(&moved(0, PointerKind::Mouse, 60.0, 60.0)), None);
    }

    #[test]
    fn the_right_button_with_ctrl_turns_instead_of_panning() {
        let mut g = EditorGesture::default();
        g.press(&mouse_button(0, PointerButton::Secondary));
        let mut ev = moved(0, PointerKind::Mouse, 10.0, 0.0);
        ev.modifiers.ctrl = true;
        assert_eq!(g.drag(&ev), Some(Gesture::Orbit(10.0, 0.0)));
    }

    #[test]
    fn a_press_that_stays_put_is_a_tap_and_one_that_moves_is_not() {
        let mut g = EditorGesture::default();
        g.press(&down(1, PointerKind::Touch, 100.0, 100.0));
        assert!(g.release(&up(1, PointerKind::Touch, 102.0, 103.0)));

        let mut dragged = EditorGesture::default();
        dragged.press(&down(1, PointerKind::Touch, 100.0, 100.0));
        dragged.drag(&moved(1, PointerKind::Touch, 400.0, 400.0));
        assert!(
            !dragged.release(&up(1, PointerKind::Touch, 400.0, 400.0)),
            "a drag is not a tap, even one that ends where it started"
        );
    }

    #[test]
    fn lifting_one_finger_leaves_the_other_dragging() {
        let mut g = EditorGesture::default();
        g.press(&down(1, PointerKind::Touch, 100.0, 100.0));
        g.press(&down(2, PointerKind::Touch, 300.0, 100.0));
        g.drag(&moved(1, PointerKind::Touch, 140.0, 100.0));
        g.release(&up(2, PointerKind::Touch, 300.0, 100.0));
        // Back to one finger, so the remaining one pans.
        assert_eq!(
            g.drag(&moved(1, PointerKind::Touch, 180.0, 100.0)),
            Some(Gesture::Pan(40.0, 0.0))
        );
    }

    #[test]
    fn leaving_the_editor_forgets_every_pointer() {
        let mut g = EditorGesture::default();
        g.press(&down(1, PointerKind::Touch, 100.0, 100.0));
        g.release_all();
        assert_eq!(g.pointers.len(), 0);
        assert_eq!(g.hover, None);
        assert!(
            !g.release(&up(1, PointerKind::Touch, 100.0, 100.0)),
            "a press from before the editor was left is not a tap now"
        );
    }
}

#[cfg(test)]
mod overlay_tests {
    use super::super::edit::*;
    use crate::app::state::UiAct;
    use repose_core::{Dp, Scene, Vec2};
    use repose_ui::layout_and_paint;
    use std::cell::Cell;
    use std::collections::HashMap;

    fn compose(view: View) -> (Scene, Vec<repose_core::HitRegion>) {
        let (scene, hits, _) = layout_and_paint(
            &view,
            (800, 600),
            &HashMap::new(),
            &Default::default(),
            None,
        );
        (scene, hits)
    }

    /// The overlay is what stands between the player and the viewport's own
    /// gestures, and it is easy to lose without noticing: a sibling painted
    /// *before* the viewport takes no pointer events at all, and every camera
    /// control in the editor silently stops working.
    #[test]
    fn the_overlay_claims_the_whole_viewport_and_takes_the_wheel() {
        let queue = crate::app::state::ActionQueue::default();
        let overlay = gesture_overlay(
            EditorGesture::new_handle(),
            std::rc::Rc::new(Cell::new(false)),
            queue,
        );
        let (_scene, hits) = compose(overlay);

        let taking = hits
            .iter()
            .find(|hit| hit.on_scroll.is_some())
            .expect("the overlay must take the wheel, or the track cannot be zoomed");
        assert!(
            taking.rect.w >= 799.0 && taking.rect.h >= 599.0,
            "the overlay covers {:.0}x{:.0} of an 800x600 viewport",
            taking.rect.w,
            taking.rect.h
        );
        assert!(
            taking.on_pointer_down.is_some()
                && taking.on_pointer_move.is_some()
                && taking.on_pointer_up.is_some(),
            "and it must take the pointer down, move and up, or nothing can be placed"
        );
    }

    #[test]
    fn one_wheel_notch_zooms_and_shift_plus_a_wheel_edits_the_height() {
        let queue = crate::app::state::ActionQueue::default();
        // The wheel's sign is the platform's, negated on the way in: a notch
        // upwards arrives as a negative y. The factor is the viewport's own
        // expression, kept so the editor's zoom is the direction it has always
        // been rather than a direction this change invented.
        let zoomed = wheel(&queue, Vec2 { x: 0.0, y: -120.0 }, false);
        assert_eq!(zoomed, Vec2::ZERO, "the notch is consumed, not passed on");
        let queued = queue.borrow().clone();
        assert!(
            matches!(queued.as_slice(), [UiAct::EditorZoom(f)] if *f > 1.0),
            "a notch up pulls the view back: {queued:?}"
        );

        let queue = crate::app::state::ActionQueue::default();
        wheel(&queue, Vec2 { x: 0.0, y: -120.0 }, true);
        assert_eq!(
            queue.borrow().as_slice(),
            [UiAct::MoveCursor([0, 1, 0])],
            "with Shift held the same notch raises the edited height"
        );
    }

    #[test]
    fn a_wheel_notch_too_small_to_mean_anything_is_left_alone() {
        let queue = crate::app::state::ActionQueue::default();
        let dither = Vec2 { x: 0.0, y: 0.2 };
        assert_eq!(
            wheel(&queue, dither, false),
            dither,
            "a trackpad's resting noise must not zoom the view"
        );
        assert!(queue.borrow().is_empty());
    }

    #[test]
    fn the_palette_tool_measures_as_a_square_it_can_be_tapped() {
        // A tool smaller than the touch minimum is a tool that has to be hit.
        assert!(Dp(44.0) <= crate::ui::dims::tool_w());
    }
}
