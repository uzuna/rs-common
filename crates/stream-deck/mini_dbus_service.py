#!/usr/bin/env python3
"""
DBus 動作確認用ミニマムサービス（AT-SPI 不使用）

起動:
  /usr/bin/python3 -u crates/stream-deck/mini_dbus_service.py

確認:
  gdbus call --session \\
    --dest io.github.uzuna.StreamDeckHelper \\
    --object-path /io/github/uzuna/StreamDeckHelper \\
    --method org.freedesktop.DBus.Properties.Get \\
    io.github.uzuna.StreamDeckHelper ActiveWindowTitle
"""

import sys
import signal
import dbus
import dbus.service
import dbus.mainloop.glib
from gi.repository import GLib

SERVICE_NAME = "io.github.uzuna.StreamDeckHelper"
OBJECT_PATH = "/io/github/uzuna/StreamDeckHelper"
INTERFACE_NAME = "io.github.uzuna.StreamDeckHelper"


class MinimalService(dbus.service.Object):
    """ActiveWindowTitle プロパティだけを持つ最小 DBus サービス。"""

    def __init__(self, bus, object_path):
        super().__init__(bus, object_path)
        self._title = "hello-from-dbus"

    @dbus.service.method(
        dbus_interface="org.freedesktop.DBus.Properties",
        in_signature="ss",
        out_signature="v",
    )
    def Get(self, interface_name, property_name):
        if property_name == "ActiveWindowTitle":
            return dbus.String(self._title)
        raise dbus.exceptions.DBusException(
            f"不明なプロパティ: {interface_name}.{property_name}",
            name="org.freedesktop.DBus.Error.UnknownProperty",
        )

    @dbus.service.method(
        dbus_interface="org.freedesktop.DBus.Properties",
        in_signature="s",
        out_signature="a{sv}",
    )
    def GetAll(self, interface_name):
        if interface_name == INTERFACE_NAME:
            return {"ActiveWindowTitle": dbus.String(self._title)}
        return {}


def main():
    # DBus GLib mainloop は全接続前に設定する（これが重要）
    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)

    try:
        session_bus = dbus.SessionBus()
    except Exception as e:
        print(f"[mini] セッションバス接続失敗: {e}", file=sys.stderr, flush=True)
        sys.exit(1)

    try:
        bus_name = dbus.service.BusName(SERVICE_NAME, session_bus)
    except dbus.exceptions.NameExistsException:
        print(f"[mini] サービス名が使用中: {SERVICE_NAME}", file=sys.stderr, flush=True)
        sys.exit(1)

    service = MinimalService(session_bus, OBJECT_PATH)
    loop = GLib.MainLoop()

    def on_signal(signum, frame):
        print("[mini] 終了", file=sys.stderr, flush=True)
        loop.quit()

    signal.signal(signal.SIGTERM, on_signal)
    signal.signal(signal.SIGINT, on_signal)

    print(f"[mini] 起動完了: {SERVICE_NAME}", flush=True)
    print(f"[mini]   path     : {OBJECT_PATH}", flush=True)
    print(f"[mini]   interface: {INTERFACE_NAME}", flush=True)
    print(f"[mini]   property : ActiveWindowTitle = {service._title!r}", flush=True)
    loop.run()
    print("[mini] 終了完了", flush=True)


if __name__ == "__main__":
    main()
