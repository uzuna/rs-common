.PHONY: install test format lint bench run run-multi run-spike run-spike-noise run-dual run-dual-ann run-multimode run-servo

install:
	uv sync --extra dev

format:
	uv run ruff format src tests benchmarks

lint:
	uv run ruff check src tests benchmarks

test:
	uv run pytest -v

bench:
	uv run python benchmarks/bench_infer.py

run:
	uv run python -m esn_anomaly.waveform.train

run-multi:
	uv run python -m esn_anomaly.waveform.train_multi

run-spike:
	uv run python -m esn_anomaly.spike.train

run-spike-noise:
	uv run python -m esn_anomaly.spike.train_noise

run-dual:
	uv run python -m esn_anomaly.multichannel.train

run-dual-ann:
	uv run python -m esn_anomaly.multichannel.train_annotated

run-multimode:
	uv run python -m esn_anomaly.multimode.train

run-servo:
	uv run python -m esn_anomaly.servo.train
