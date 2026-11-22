//! The one thing the pose code asks of an inference engine: load an embedded
//! ONNX model with one f32 input of a fixed shape, run it, and hand back the
//! f32 outputs it names.
//!
//! Two back-ends, one per feature:
//! - `ort`: ONNX Runtime, through `ort`'s prebuilt binaries. The fastest, but
//!   `ort` ships them only for some targets.
//! - `tract`: `tract`, pure Rust, so it builds wherever Rust does (armv7
//!   Android, `x86_64` macOS). The same outputs to about 10⁻³ px on the
//!   landmarks, but some 2.5 times slower than ONNX Runtime on one thread
//!   (12 ms against 5 for the landmark model on an M-series core). Needs
//!   Rust 1.91, above the workspace's 1.88.
//!
//! With both features on (feature unification), ONNX Runtime is used.

use anyhow::Result;

#[cfg(not(any(feature = "ort", feature = "tract")))]
compile_error!("tobii-pose needs an inference back-end: enable its `ort` or `tract` feature");

/// Which back-end this build runs the models on.
pub const BACKEND: &str = if cfg!(feature = "ort") {
    "ONNX Runtime"
} else {
    "tract"
};

/// A loaded model: one f32 input of shape `shape`, the outputs `outputs`.
pub(crate) struct Model {
    #[cfg(feature = "ort")]
    session: ort::session::Session,
    #[cfg(all(feature = "tract", not(feature = "ort")))]
    plan: std::sync::Arc<tract_onnx::prelude::TypedRunnableModel>,
    input: &'static str,
    shape: [usize; 4],
    outputs: &'static [&'static str],
}

#[cfg(all(test, feature = "ort"))]
impl Model {
    /// The ONNX Runtime session, for the tests of the models' inputs and
    /// outputs.
    pub(crate) fn session(&self) -> &ort::session::Session {
        &self.session
    }
}

impl Model {
    /// Load the ONNX model `bytes`, fed through its input `input` of shape
    /// `shape`, and read back through its outputs `outputs`. `one_thread`
    /// keeps ONNX Runtime to one intra-op thread (tract always runs on
    /// one).
    ///
    /// # Errors
    /// Fails when the back-end cannot start or rejects the model.
    #[cfg(feature = "ort")]
    pub(crate) fn load(
        bytes: &[u8],
        input: &'static str,
        shape: [usize; 4],
        outputs: &'static [&'static str],
        one_thread: bool,
    ) -> Result<Self> {
        use anyhow::Context;
        let mut builder =
            ort::session::Session::builder().context("failed to start ONNX Runtime")?;
        if one_thread {
            // In ort 2.0.0-rc.12 the builder's setters fail with an error
            // that hands the builder back, which is not Send; convert it to
            // a plain ort::Error.
            builder = builder
                .with_intra_threads(1)
                .map_err(ort::Error::<()>::from)
                .context("failed to give the model one thread")?;
        }
        let session = builder.commit_from_memory(bytes)?;
        Ok(Self {
            session,
            input,
            shape,
            outputs,
        })
    }

    /// See the `ort` version of `load`.
    #[cfg(all(feature = "tract", not(feature = "ort")))]
    pub(crate) fn load(
        bytes: &[u8],
        input: &'static str,
        shape: [usize; 4],
        outputs: &'static [&'static str],
        _one_thread: bool,
    ) -> Result<Self> {
        use anyhow::Context;
        use tract_onnx::prelude::*;
        let model = tract_onnx::onnx()
            .model_for_read(&mut &*bytes)?
            .with_input_names([input])?
            .with_input_fact(0, f32::fact(shape).into())?;
        let outlets = outputs
            .iter()
            .map(|name| {
                model
                    .find_outlet_label(name)
                    .with_context(|| format!("the model has no {name} output"))
            })
            .collect::<Result<Vec<_>>>()?;
        let plan = model
            .with_output_outlets(&outlets)?
            .into_optimized()?
            .into_runnable()?;
        Ok(Self {
            plan,
            input,
            shape,
            outputs,
        })
    }

    /// Run the model on `data` (laid out in the input's shape) and pass
    /// its outputs, in the order of `outputs`, to `read`.
    ///
    /// # Errors
    /// Fails when `data` is not of the input's size, the run fails, or an
    /// output is missing or not f32; or with `read`'s own error.
    #[cfg(feature = "ort")]
    pub(crate) fn run<R>(
        &mut self,
        data: &[f32],
        read: impl FnOnce(&[&[f32]]) -> Result<R>,
    ) -> Result<R> {
        use anyhow::Context;
        let value = ort::value::TensorRef::from_array_view((self.shape, data))
            .context("passing the input to the model")?;
        let out = self.session.run(ort::inputs![self.input => value])?;
        let mut slices = Vec::with_capacity(self.outputs.len());
        for name in self.outputs {
            let (_, data) = out
                .get(name)
                .with_context(|| format!("the model has no {name} output"))?
                .try_extract_tensor::<f32>()
                .with_context(|| format!("reading the model's {name} output"))?;
            slices.push(data);
        }
        read(&slices)
    }

    /// See the `ort` version of `run`.
    #[cfg(all(feature = "tract", not(feature = "ort")))]
    pub(crate) fn run<R>(
        &mut self,
        data: &[f32],
        read: impl FnOnce(&[&[f32]]) -> Result<R>,
    ) -> Result<R> {
        use anyhow::Context;
        use tract_onnx::prelude::*;
        let _ = self.input;
        let tensor =
            Tensor::from_shape(&self.shape, data).context("passing the input to the model")?;
        let out = self.plan.run(tvec!(tensor.into()))?;
        anyhow::ensure!(
            out.len() == self.outputs.len(),
            "the model returned {} outputs, not {}",
            out.len(),
            self.outputs.len()
        );
        let mut slices = Vec::with_capacity(out.len());
        for (value, name) in out.iter().zip(self.outputs) {
            let view = value
                .to_plain_array_view::<f32>()
                .with_context(|| format!("reading the model's {name} output"))?;
            slices.push(
                view.to_slice()
                    .with_context(|| format!("the model's {name} output is not contiguous"))?,
            );
        }
        read(&slices)
    }
}
