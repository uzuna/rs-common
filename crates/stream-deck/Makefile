REPO_ROOT := $(shell git rev-parse --show-toplevel)
SYSTEMD_USER_DIR := $(HOME)/.config/systemd/user

SERVICES := stream-deck stream-deck-tracker

# ─── ビルド ───────────────────────────────────────────────────────────────────

.PHONY: build
build:
	cargo build --release -p stream-deck

# ─── サービス管理 ─────────────────────────────────────────────────────────────

# サービスユニットを生成してインストールする（初回のみ・ビルド後に実行）
.PHONY: install-service
install-service:
	mkdir -p $(SYSTEMD_USER_DIR)
	@for svc in $(SERVICES); do \
		sed 's|%%REPO_ROOT%%|$(REPO_ROOT)|g' \
			deploy/$$svc.service > $(SYSTEMD_USER_DIR)/$$svc.service; \
		echo "インストール: $(SYSTEMD_USER_DIR)/$$svc.service"; \
	done
	systemctl --user daemon-reload
	systemctl --user enable $(SERVICES)
	@echo "登録完了。'make start' で起動してください"

# サービスユニットを削除する
.PHONY: uninstall-service
uninstall-service:
	-systemctl --user disable --now $(SERVICES)
	@for svc in $(SERVICES); do \
		rm -f $(SYSTEMD_USER_DIR)/$$svc.service; \
		echo "削除: $(SYSTEMD_USER_DIR)/$$svc.service"; \
	done
	systemctl --user daemon-reload

# ビルドしてサービスを再起動する
.PHONY: deploy
deploy: build
	systemctl --user restart $(SERVICES)
	@echo "デプロイ完了"

# サービスの起動 / 停止 / 状態確認 / ログ
.PHONY: start stop status log
start:
	systemctl --user start $(SERVICES)

stop:
	systemctl --user stop $(SERVICES)

status:
	systemctl --user status $(SERVICES)

log:
	journalctl --user -u stream-deck -u stream-deck-tracker -f
