.PHONY: bench-json-baseline-save bench-json-baseline-compare bench-json-baseline-refresh

# baseline名は固定運用
BASELINE_NAME := healed-serde-json-baseline
SAMPLE_SIZE ?= 30

# 現在実装をbaselineとして保存
bench-json-baseline-save:
	cargo bench -p healed-serde --bench json_serialize_bench -- --save-baseline $(BASELINE_NAME) --sample-size $(SAMPLE_SIZE)

# 固定baselineとの差分を計測
bench-json-baseline-compare:
	cargo bench -p healed-serde --bench json_serialize_bench -- --baseline $(BASELINE_NAME) --sample-size $(SAMPLE_SIZE)

# baselineの更新（保存→比較）
bench-json-baseline-refresh: bench-json-baseline-save bench-json-baseline-compare
