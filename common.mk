# 共通の環境変数設定
mkfile_path := $(abspath $(lastword $(MAKEFILE_LIST)))
PROJECT_DIR := $(patsubst %/,%,$(dir $(mkfile_path)))

ASSETS_DIR := $(PROJECT_DIR)/examples/preview-server/assets
