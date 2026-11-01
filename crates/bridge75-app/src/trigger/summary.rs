//! What the selected keys have in common, field by field. No GPUI.

use bridge75_core::he::{Travel, TriggerConfig, TriggerMode};
use bridge75_core::matrix::KeyPos;

use crate::document::{TriggerField, TriggerTable, stepped};

/// One field across the selected keys whose mode uses it. Keys that do not use the field are
/// left out, since its value has no effect on them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldSummary {
    values: Vec<Travel>,
}

impl FieldSummary {
    /// The value of the first such key, which places the slider.
    pub fn first(&self) -> Travel {
        self.values[0]
    }

    /// The value every such key has, if they agree.
    pub fn same(&self) -> Option<Travel> {
        let first = self.first();
        self.values.iter().all(|v| *v == first).then_some(first)
    }

    pub fn is_mixed(&self) -> bool {
        self.same().is_none()
    }

    /// Whether stepping by `delta` would change any key.
    pub fn is_steppable(&self, delta: i8) -> bool {
        self.values.iter().any(|v| stepped(*v, delta) != *v)
    }
}

pub struct SelectionSummary {
    /// Distinct modes of the selected keys, in selection order.
    modes: Vec<TriggerMode>,
    /// Indexed by [`TriggerField::ix`]; `None` where no selected key uses the field.
    fields: [Option<FieldSummary>; 4],
}

impl SelectionSummary {
    /// `None` if no selected key has settings.
    pub fn new(table: &TriggerTable, keys: &[KeyPos]) -> Option<Self> {
        let configs: Vec<TriggerConfig> = keys
            .iter()
            .filter_map(|pos| table.get(*pos))
            .copied()
            .collect();
        if configs.is_empty() {
            return None;
        }
        let mut modes: Vec<TriggerMode> = Vec::new();
        for config in &configs {
            if !modes.contains(&config.mode) {
                modes.push(config.mode);
            }
        }
        let fields = TriggerField::ALL.map(|field| {
            let values: Vec<Travel> = configs
                .iter()
                .filter(|c| field.is_used_in(c.mode))
                .map(|c| field.get(c))
                .collect();
            (!values.is_empty()).then_some(FieldSummary { values })
        });
        Some(SelectionSummary { modes, fields })
    }

    pub fn modes(&self) -> &[TriggerMode] {
        &self.modes
    }

    /// The mode every selected key has, if they agree.
    pub fn mode(&self) -> Option<TriggerMode> {
        match self.modes[..] {
            [mode] => Some(mode),
            _ => None,
        }
    }

    pub fn field(&self, field: TriggerField) -> Option<&FieldSummary> {
        self.fields[field.ix()].as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::document::MIN_TRAVEL;

    fn pos(id: &str) -> KeyPos {
        id.parse().unwrap()
    }

    fn mm(tenths: u8) -> Travel {
        Travel::new(tenths).unwrap()
    }

    fn config(mode: TriggerMode, actuation: u8, release: u8, rt: u8) -> TriggerConfig {
        TriggerConfig {
            mode,
            actuation: Travel::from_raw(actuation),
            release: Travel::from_raw(release),
            rt_press: Travel::from_raw(rt),
            rt_release: Travel::from_raw(rt),
        }
    }

    fn summary(configs: &[(&str, TriggerConfig)]) -> SelectionSummary {
        let table = TriggerTable::new(configs.iter().map(|(id, c)| (pos(id), *c)), 0);
        let keys: Vec<KeyPos> = configs.iter().map(|(id, _)| pos(id)).collect();
        SelectionSummary::new(&table, &keys).unwrap()
    }

    #[test]
    fn fields_only_count_keys_that_use_them() {
        // The Normal key's RT sensitivity was never set and must not make the field look mixed.
        let summary = summary(&[
            ("w", config(TriggerMode::Normal, 18, 18, 0)),
            ("a", config(TriggerMode::Rapid, 10, 10, 3)),
        ]);
        assert_eq!(summary.mode(), None);
        let rt = summary.field(TriggerField::RtPress).unwrap();
        assert_eq!((rt.same(), rt.first()), (Some(mm(3)), mm(3)));
        assert!(summary.field(TriggerField::Release).is_none());
        let actuation = summary.field(TriggerField::Actuation).unwrap();
        assert!(actuation.is_mixed());
        assert_eq!(actuation.first(), mm(18));
    }

    #[test]
    fn the_release_point_counts_only_modes_with_one() {
        let summary = summary(&[
            ("w", config(TriggerMode::Normal, 18, 18, 5)),
            ("a", config(TriggerMode::Split, 18, 8, 5)),
            ("s", config(TriggerMode::RapidSplit, 18, 8, 5)),
        ]);
        let release = summary.field(TriggerField::Release).unwrap();
        assert_eq!(release.same(), Some(mm(8)));
        assert_eq!(summary.modes().len(), 3);
    }

    #[test]
    fn values_outside_the_range_are_shown_as_stored() {
        let summary = summary(&[("w", config(TriggerMode::Rapid, 18, 18, 200))]);
        let rt = summary.field(TriggerField::RtPress).unwrap();
        assert_eq!(rt.same(), Some(Travel::from_raw(200)));
        // Both directions bring it back into range.
        assert!(rt.is_steppable(-1) && rt.is_steppable(1));
    }

    #[test]
    fn steps_are_offered_while_any_key_can_move() {
        let summary = summary(&[
            ("w", config(TriggerMode::Normal, MIN_TRAVEL.raw(), 1, 5)),
            ("a", config(TriggerMode::Normal, MIN_TRAVEL.raw(), 1, 5)),
        ]);
        let actuation = summary.field(TriggerField::Actuation).unwrap();
        assert!(!actuation.is_steppable(-1));
        assert!(actuation.is_steppable(1));

        let summary = summary_at_max();
        let actuation = summary.field(TriggerField::Actuation).unwrap();
        assert!(actuation.is_steppable(-1));
        assert!(!actuation.is_steppable(1));
    }

    fn summary_at_max() -> SelectionSummary {
        summary(&[("w", config(TriggerMode::Normal, Travel::MAX.raw(), 36, 5))])
    }

    #[test]
    fn a_single_mode_is_reported() {
        let summary = summary(&[
            ("w", config(TriggerMode::Rapid, 18, 18, 5)),
            ("a", config(TriggerMode::Rapid, 10, 10, 5)),
        ]);
        assert_eq!(summary.mode(), Some(TriggerMode::Rapid));
    }
}
