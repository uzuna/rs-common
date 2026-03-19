#![allow(non_local_definitions)]

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyModule};
use rhythm_core::{
    BpmLimitParam as BpmLimitParamRs, BpmQ8, PulseSyncParam as PulseSyncParamRs,
    RhythmGenerator as RhythmGeneratorRs, RhythmMessage as RhythmMessageRs,
    SyncState as SyncStateRs, BPM_Q8_ONE,
};

// --- Enums ---

#[pyclass(name = "SyncState", skip_from_py_object)]
#[derive(Clone, Copy)]
pub enum PySyncState {
    Idle,
    WaitSecondPoint,
    Locked,
}

impl From<SyncStateRs> for PySyncState {
    fn from(state: SyncStateRs) -> Self {
        match state {
            SyncStateRs::Idle => PySyncState::Idle,
            SyncStateRs::WaitSecondPoint => PySyncState::WaitSecondPoint,
            SyncStateRs::Locked => PySyncState::Locked,
        }
    }
}

// --- Param Structs ---

#[pyclass(name = "BpmLimitParam", skip_from_py_object)]
#[derive(Clone, Copy)]
pub struct PyBpmLimitParam {
    pub inner: BpmLimitParamRs,
}

#[pymethods]
impl PyBpmLimitParam {
    #[new]
    fn new(min_bpm: u16, max_bpm: u16) -> Self {
        Self {
            inner: BpmLimitParamRs::new(min_bpm, max_bpm),
        }
    }
}

#[pyclass(name = "PulseSyncParam", skip_from_py_object)]
#[derive(Clone, Copy)]
pub struct PyPulseSyncParam {
    pub inner: PulseSyncParamRs,
}

#[pymethods]
impl PyPulseSyncParam {
    #[new]
    fn new(missing_cycle_threshold: u8, bpm_ema_alpha_q8: u8) -> Self {
        Self {
            inner: PulseSyncParamRs::new(missing_cycle_threshold, bpm_ema_alpha_q8),
        }
    }
}

// --- Core Structs ---

#[pyclass(name = "RhythmMessage", skip_from_py_object)]
#[derive(Clone)]
pub struct PyRhythmMessage {
    pub inner: RhythmMessageRs,
}

#[pymethods]
impl PyRhythmMessage {
    #[new]
    fn new(timestamp_ms: u64, beat_count: u32, phase: u16, bpm_raw: u16) -> Self {
        PyRhythmMessage {
            inner: RhythmMessageRs::new(timestamp_ms, beat_count, phase, BpmQ8(bpm_raw)),
        }
    }

    #[getter]
    fn timestamp_ms(&self) -> u64 {
        self.inner.timestamp_ms
    }
    #[getter]
    fn beat_count(&self) -> u32 {
        self.inner.beat_count
    }
    #[getter]
    fn phase(&self) -> u16 {
        self.inner.phase
    }
    #[getter]
    fn bpm_raw(&self) -> u16 {
        self.inner.bpm.raw()
    }

    fn to_wire_bytes<'a>(&self, py: Python<'a>) -> Bound<'a, PyBytes> {
        PyBytes::new(py, &self.inner.to_wire_bytes())
    }

    #[staticmethod]
    fn from_wire_slice(bytes: &[u8]) -> PyResult<Self> {
        RhythmMessageRs::from_wire_slice(bytes)
            .map(|inner| Self { inner })
            .ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "Invalid byte slice for RhythmMessage",
                )
            })
    }

    fn __str__(&self) -> String {
        format!("{:?}", self.inner)
    }
    fn __repr__(&self) -> String {
        self.__str__()
    }
}

#[pyclass(name = "RhythmGenerator")]
pub struct PyRhythmGenerator {
    pub inner: RhythmGeneratorRs,
}

#[pymethods]
impl PyRhythmGenerator {
    #[new]
    fn new(phase: u16, base_bpm_q8: u16, coupling_divisor: u16) -> Self {
        Self {
            inner: RhythmGeneratorRs::new(phase, BpmQ8(base_bpm_q8), coupling_divisor),
        }
    }

    #[staticmethod]
    fn from_int_bpm(phase: u16, bpm: u16, coupling_divisor: u16) -> Self {
        Self {
            inner: RhythmGeneratorRs::from_int_bpm(phase, bpm, coupling_divisor),
        }
    }

    fn update(&mut self, dt_ms: u32, bpm_limit_param: &PyBpmLimitParam) {
        self.inner.update(dt_ms, &bpm_limit_param.inner);
    }

    fn sync(&mut self, msg: &PyRhythmMessage, now_ms: u64, bpm_limit_param: &PyBpmLimitParam) {
        self.inner.sync(msg.inner, now_ms, &bpm_limit_param.inner);
    }

    fn sync_pulse(&mut self, pulse_ts_ms: u64, now_ms: u64, bpm_limit_param: &PyBpmLimitParam) {
        self.inner
            .sync_pulse(pulse_ts_ms, now_ms, &bpm_limit_param.inner);
    }

    fn to_message(&self, now_ms: u64) -> PyRhythmMessage {
        PyRhythmMessage {
            inner: self.inner.to_message(now_ms),
        }
    }

    fn set_pulse_sync_param(&mut self, param: &PyPulseSyncParam) {
        self.inner.set_pulse_sync_param(param.inner);
    }

    #[getter]
    fn pulse_sync_param(&self) -> PyPulseSyncParam {
        PyPulseSyncParam {
            inner: self.inner.pulse_sync_param(),
        }
    }

    #[getter]
    fn phase(&self) -> u16 {
        self.inner.phase
    }
    #[getter]
    fn beat_count(&self) -> u64 {
        self.inner.beat_count
    }
    #[getter]
    fn sync_state(&self) -> PySyncState {
        self.inner.sync_state.into()
    }
    #[getter]

    fn base_bpm_raw(&self) -> u16 {
        self.inner.base_bpm.raw()
    }
    #[getter]
    fn current_bpm_raw(&self) -> u16 {
        self.inner.current_bpm.raw()
    }
}

// --- Module Definition ---

#[pymodule]
fn py_rhythm_core(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySyncState>()?;
    m.add_class::<PyBpmLimitParam>()?;
    m.add_class::<PyPulseSyncParam>()?;
    m.add_class::<PyRhythmMessage>()?;
    m.add_class::<PyRhythmGenerator>()?;

    m.add("BPM_Q8_ONE", BPM_Q8_ONE)?;

    #[pyfunction]
    #[pyo3(name = "bpm_q8_from_int")]
    fn bpm_q8_from_int_py(_py: Python, bpm: u16) -> u16 {
        BpmQ8::from_int(bpm).raw()
    }

    #[pyfunction]
    #[pyo3(name = "bpm_q8_to_float")]
    fn bpm_q8_to_float_py(_py: Python, bpm_q8: u16) -> f64 {
        BpmQ8(bpm_q8).to_float()
    }

    m.add_function(pyo3::wrap_pyfunction!(bpm_q8_from_int_py, m)?)?;
    m.add_function(pyo3::wrap_pyfunction!(bpm_q8_to_float_py, m)?)?;

    Ok(())
}
