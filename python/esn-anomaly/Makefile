.PHONY: install test format lint bench run

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
