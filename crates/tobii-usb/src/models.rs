//! Supported Tobii tracker models and the USB device registry.
//!
//! Every Tobii tracker this crate knows about shares one USB vendor id
//! ([`VID`]) and is told apart by its product id. The ET5 is the production
//! model this crate was built around; the Pro Spark is recognised on the bus
//! and reported, but cannot yet stream — its init replay has not been captured
//! (see the gaps in `docs/spark.md`).
//!
//! The device's *model name* as a client sees it (e.g. `IS5FF`, `TPE01`) is not
//! taken from here: the tracker reports it over TTP and it surfaces through
//! `DeviceFacts`. This registry is only about **recognising the USB device and
//! choosing the transport/init profile** for it. [`Model::serial_prefix`] is a
//! hint for humans and logs, not an identity check.

/// USB vendor id shared by every supported Tobii tracker (Tobii AB).
pub const VID: u16 = 0x2104;

/// Interface carrying the processed-stream endpoints. Shared by the known
/// models (both expose a single vendor-class interface 0 with three
/// endpoints).
pub const IFACE: u8 = 0;

/// Bulk IN endpoint of the multiplexed processed stream.
pub const EP_IN: u8 = 0x83;

/// Bulk OUT endpoint for command messages (what the init replay writes to).
pub const EP_OUT: u8 = 0x05;

/// A Tobii tracker model this crate can recognise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Model {
    /// Tobii Eye Tracker 5 — the consumer tracker (`IS5`/`IS5FF` firmware).
    /// Fully supported: the embedded init capture drives it.
    EyeTracker5,
    /// Tobii Pro Spark — the entry-level Pro tracker (`TPE01` serial,
    /// `TPSP1` firmware, platform codename *Castor*). Recognised on the bus,
    /// but not yet drivable: no init replay has been captured for it.
    ProSpark,
}

/// Where this crate gets the init replay for a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitSource {
    /// The init sequence is embedded in the crate (`init_packets_ep.txt`).
    Embedded,
    /// No init replay is available yet: the device is recognised but cannot
    /// be configured or streamed. Carries a short note on what is missing.
    Unavailable(&'static str),
}

/// Static facts about one supported model: how to recognise its USB device and
/// which transport/init profile to use for it.
#[derive(Debug, Clone, Copy)]
pub struct ModelInfo {
    /// The model this entry describes.
    pub model: Model,
    /// USB product id (`idProduct`) under [`VID`].
    pub pid: u16,
    /// Human-facing device name for logs and messages.
    pub display_name: &'static str,
    /// The serial-number prefix the device reports (a hint, not an identity
    /// check — the real model comes from the device over TTP).
    pub serial_prefix: &'static str,
    /// USB product string the device presents (`iProduct`). Both known models
    /// present `"EyeChip"`, so it does not distinguish them; kept for
    /// documentation and sanity logging.
    pub product_string: &'static str,
    /// Interface to claim.
    pub iface: u8,
    /// Bulk IN endpoint (processed stream).
    pub ep_in: u8,
    /// Bulk OUT endpoint (command messages / init replay).
    pub ep_out: u8,
    /// Where the init replay for this model comes from.
    pub init: InitSource,
}

impl ModelInfo {
    /// Whether this model can be configured and streamed (has an init replay).
    #[must_use]
    pub const fn is_drivable(&self) -> bool {
        matches!(self.init, InitSource::Embedded)
    }
}

/// The registry of supported models.
///
/// ET5 is fully populated. Spark carries its confirmed USB facts (VID/PID
/// `2104:031C`, product `"EyeChip"`, one vendor-class interface with three
/// endpoints — the same transport topology as the ET5) and, pending a capture,
/// an [`InitSource::Unavailable`]. The Spark endpoint addresses are assumed to
/// match the ET5 (bulk `0x05` OUT / `0x83` IN) from the shared Castor
/// TTP-over-USB transport; verify against a real capture before relying on them.
pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        model: Model::EyeTracker5,
        pid: 0x0313,
        display_name: "Tobii Eye Tracker 5",
        serial_prefix: "IS5FF",
        product_string: "EyeChip",
        iface: IFACE,
        ep_in: EP_IN,
        ep_out: EP_OUT,
        init: InitSource::Embedded,
    },
    ModelInfo {
        model: Model::ProSpark,
        pid: 0x031C,
        display_name: "Tobii Pro Spark",
        serial_prefix: "TPE01",
        product_string: "EyeChip",
        iface: IFACE,
        ep_in: EP_IN,
        ep_out: EP_OUT,
        init: InitSource::Unavailable(
            "no init replay captured for the Pro Spark yet; see docs/spark.md",
        ),
    },
];

/// The model for a `(vid, pid)` pair, if it is a supported Tobii tracker.
#[must_use]
pub fn recognise(vid: u16, pid: u16) -> Option<&'static ModelInfo> {
    if vid != VID {
        return None;
    }
    MODELS.iter().find(|m| m.pid == pid)
}

/// Facts about a known [`Model`].
///
/// # Panics
///
/// Never, in practice: every [`Model`] variant has a [`MODELS`] entry, which
/// the `every_model_has_an_entry` test enforces. Panics only if that invariant
/// is broken.
#[must_use]
pub fn info(model: Model) -> &'static ModelInfo {
    MODELS
        .iter()
        .find(|m| m.model == model)
        .expect("every Model has a registry entry")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_the_et5() {
        let m = recognise(0x2104, 0x0313).expect("ET5 recognised");
        assert_eq!(m.model, Model::EyeTracker5);
        assert!(m.is_drivable());
    }

    #[test]
    fn recognises_the_spark_but_it_is_not_drivable() {
        let m = recognise(0x2104, 0x031C).expect("Spark recognised");
        assert_eq!(m.model, Model::ProSpark);
        assert_eq!(m.serial_prefix, "TPE01");
        assert!(!m.is_drivable());
    }

    #[test]
    fn rejects_other_vendors_and_unknown_pids() {
        assert!(recognise(0x1234, 0x0313).is_none());
        assert!(recognise(0x2104, 0xFFFF).is_none());
    }

    #[test]
    fn every_model_has_an_entry() {
        // info() must not panic for any Model variant.
        for m in [Model::EyeTracker5, Model::ProSpark] {
            assert_eq!(info(m).model, m);
        }
    }

    #[test]
    fn pids_are_unique() {
        for (i, a) in MODELS.iter().enumerate() {
            for b in &MODELS[i + 1..] {
                assert_ne!(a.pid, b.pid, "duplicate PID {:#06x}", a.pid);
            }
        }
    }
}
