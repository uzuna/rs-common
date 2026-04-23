.PHONY: install test format lint bench run run-multi run-spike run-spike-noise run-dual

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
	uv run python -m esn_anomaly.train

run-multi:
	uv run python -m esn_anomaly.train_multi

run-spike:
	uv run python -m esn_anomaly.train_spike

run-spike-noise:
	uv run python -m esn_anomaly.train_spike_noise

run-dual:
	uv run python -m esn_anomaly.train_dual
