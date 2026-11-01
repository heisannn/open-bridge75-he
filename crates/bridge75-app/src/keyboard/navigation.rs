//! Moving between keys with the arrow keys, following where the keys sit. No GPUI.

use bridge75_core::matrix::{KeyPos, KeyRect, LAYOUT};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Horizontal center, in eighths of a unit so it stays an integer.
fn center_x(rect: KeyRect) -> i16 {
    2 * rect.x as i16 + rect.w as i16
}

/// The key next to `from` in `direction`, or `None` at the edge of the board.
///
/// Left and right stay in the same row. Up and down go to the nearest row that way and pick
/// the key whose center is closest to that of `from`.
pub fn neighbor(from: KeyPos, direction: Direction) -> Option<KeyPos> {
    let rect = from.key()?.rect;
    let center = center_x(rect);
    let distance = |other: KeyRect| (center_x(other) - center).abs();
    match direction {
        Direction::Left | Direction::Right => {
            let toward = |other: KeyRect| match direction {
                Direction::Left => center_x(other) < center,
                _ => center_x(other) > center,
            };
            LAYOUT
                .iter()
                .filter(|key| key.rect.y == rect.y && toward(key.rect))
                .min_by_key(|key| distance(key.rect))
                .map(|key| key.pos)
        }
        Direction::Up | Direction::Down => {
            let row = |other: KeyRect| match direction {
                Direction::Up => (other.y < rect.y).then(|| rect.y - other.y),
                _ => (other.y > rect.y).then(|| other.y - rect.y),
            };
            let nearest = LAYOUT.iter().filter_map(|key| row(key.rect)).min()?;
            LAYOUT
                .iter()
                .filter(|key| row(key.rect) == Some(nearest))
                .min_by_key(|key| distance(key.rect))
                .map(|key| key.pos)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(id: &str) -> KeyPos {
        LAYOUT.iter().find(|k| k.id == id).unwrap().pos
    }

    fn go(id: &str, direction: Direction) -> Option<&'static str> {
        let to = neighbor(pos(id), direction)?;
        Some(to.key().unwrap().id)
    }

    #[test]
    fn left_and_right_stay_in_the_row() {
        assert_eq!(go("w", Direction::Right), Some("e"));
        assert_eq!(go("w", Direction::Left), Some("q"));
        assert_eq!(go("f12", Direction::Right), Some("del"));
        assert_eq!(go("esc", Direction::Left), None);
        assert_eq!(go("right", Direction::Right), None);
    }

    #[test]
    fn up_and_down_pick_the_closest_key_in_the_next_row() {
        assert_eq!(go("space", Direction::Up), Some("b"));
        assert_eq!(go("b", Direction::Down), Some("space"));
        assert_eq!(go("up", Direction::Down), Some("down"));
        assert_eq!(go("del", Direction::Down), Some("home"));
        assert_eq!(go("end", Direction::Down), Some("right"));
        assert_eq!(go("esc", Direction::Up), None);
        assert_eq!(go("lctrl", Direction::Down), None);
    }

    #[test]
    fn every_key_is_reachable() {
        let mut seen = vec![LAYOUT[0].pos];
        let mut queue = seen.clone();
        while let Some(from) = queue.pop() {
            for direction in [
                Direction::Left,
                Direction::Right,
                Direction::Up,
                Direction::Down,
            ] {
                if let Some(to) = neighbor(from, direction)
                    && !seen.contains(&to)
                {
                    seen.push(to);
                    queue.push(to);
                }
            }
        }
        assert_eq!(seen.len(), LAYOUT.len());
    }
}
