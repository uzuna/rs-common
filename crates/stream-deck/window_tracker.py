#!/usr/bin/python3
"""
stream-deck 用ウィンドウタイトル追跡サービス

AT-SPI でアクティブウィンドウのタイトルを監視し、DBus プロパティとして公開する。

使い方:
  /usr/bin/python3 -u crates/stream-deck/window_tracker.py &

接続情報:
  Service  : io.github.uzuna.StreamDeckHelper
  Path     : /io/github/uzuna/StreamDeckHelper
  Interface: io.github.uzuna.StreamDeckHelper
  Property : ActiveWindowTitle

stream-deck 起動時の環境変数:
  export STREAM_DECK_DBUS_SERVICE="io.github.uzuna.StreamDeckHelper"
  export STREAM_DECK_DBUS_PATH="/io/github/uzuna/StreamDeckHelper"
  export STREAM_DECK_DBUS_INTERFACE="io.github.uzuna.StreamDeckHelper"
  export STREAM_DECK_DBUS_PROPERTY="ActiveWindowTitle"
"""

import sys
import signal

import gi
gi.require_version('Atspi', '2.0')
from gi.repository import Atspi, GLib
import dbus
import dbus.service
import dbus.mainloop.glib

# DBus サービス定数
SERVICE_NAME = "io.github.uzuna.StreamDeckHelper"
OBJECT_PATH = "/io/github/uzuna/StreamDeckHelper"
INTERFACE_NAME = "io.github.uzuna.StreamDeckHelper"


def get_active_window_title() -> str:
    """AT-SPI ツリーを走査してアクティブウィンドウのタイトルを返す。見つからなければ空文字列。"""
    try:
        desktop = Atspi.get_desktop(0)
        count = desktop.get_child_count()
        for i in range(count):
            app = desktop.get_child_at_index(i)
            if app is None:
                continue
            try:
                child_count = app.get_child_count()
                for j in range(child_count):
                    win = app.get_child_at_index(j)
                    if win is None:
                        continue
                    try:
                        state = win.get_state_set()
                        if state.contains(Atspi.StateType.ACTIVE):
                            name = win.get_name()
                            return name if name else ""
                    except Exception:
                        continue
            except Exception:
                continue
    except Exception as e:
        print(f"[window_tracker] AT-SPI エラー: {e}", file=sys.stderr)
    return ""


class WindowTrackerService(dbus.service.Object):
    """DBus プロパティ経由でアクティブウィンドウタイトルを公開するサービス。"""

    def __init__(self, bus, object_path):
        super().__init__(bus, object_path)
        self._active_title = ""

    def update_title(self):
        """AT-SPI からタイトルを取得して更新する。GLib タイマーから呼ぶ。"""
        title = get_active_window_title()
        if title != self._active_title:
            self._active_title = title
            print(f"[window_tracker] active window: {title!r}")
            try:
                self.PropertiesChanged(INTERFACE_NAME, {"ActiveWindowTitle": title}, [])
            except Exception:
                pass
        return True  # GLib タイマー継続

    @dbus.service.method(
        dbus_interface="org.freedesktop.DBus.Properties",
        in_signature="ss",
        out_signature="v",
    )
    def Get(self, interface_name, property_name):
        if interface_name == INTERFACE_NAME and property_name == "ActiveWindowTitle":
            return dbus.String(self._active_title)
        raise dbus.exceptions.DBusException(
            f"プロパティが見つかりません: {interface_name}.{property_name}",
            name="org.freedesktop.DBus.Error.UnknownProperty",
        )

    @dbus.service.method(
        dbus_interface="org.freedesktop.DBus.Properties",
        in_signature="s",
        out_signature="a{sv}",
    )
    def GetAll(self, interface_name):
        if interface_name == INTERFACE_NAME:
            return {"ActiveWindowTitle": dbus.String(self._active_title)}
        return {}

    @dbus.service.signal(
        dbus_interface="org.freedesktop.DBus.Properties",
        signature="sa{sv}as",
    )
    def PropertiesChanged(self, interface_name, changed_properties, invalidated_properties):
        pass


def main():
    # ファイルリダイレクト時もログが即時書き出されるよう line-buffered に変更する
    sys.stdout.reconfigure(line_buffering=True)

    # DBus GLib mainloop は全接続・AT-SPI 初期化より前に設定する
    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)

    # AT-SPI 初期化は mainloop 設定後に行う
    Atspi.init()

    try:
        session_bus = dbus.SessionBus()
    except Exception as e:
        print(f"[window_tracker] DBus セッションバス接続失敗: {e}", file=sys.stderr)
        sys.exit(1)

    try:
        # do_not_queue=True: 名前が使用中なら即失敗（キュー待ちしない）
        bus_name = dbus.service.BusName(SERVICE_NAME, session_bus, do_not_queue=True)
    except dbus.exceptions.NameExistsException:
        print(f"[window_tracker] サービス名がすでに使用されています: {SERVICE_NAME}", file=sys.stderr)
        sys.exit(1)

    service = WindowTrackerService(session_bus, OBJECT_PATH)
    loop = GLib.MainLoop()

    # 500ms ごとにアクティブウィンドウタイトルを更新
    GLib.timeout_add(500, service.update_title)

    def on_sigterm(signum, frame):
        print("[window_tracker] 終了シグナルを受信", file=sys.stderr)
        loop.quit()

    signal.signal(signal.SIGTERM, on_sigterm)
    signal.signal(signal.SIGINT, on_sigterm)

    print(f"[window_tracker] 起動: {SERVICE_NAME} → {OBJECT_PATH}.{INTERFACE_NAME}.ActiveWindowTitle")
    loop.run()
    print("[window_tracker] 終了")


if __name__ == "__main__":
    main()
