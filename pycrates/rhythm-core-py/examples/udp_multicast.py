import socket
import struct
import time
import argparse

from py_rhythm_core import (
    RhythmGenerator,
    RhythmMessage,
    BpmLimitParam,
    bpm_q8_to_float,
    SyncState,
)

# Match crates/rhythm-core/src/comm.rs defaults.
MCAST_GROUP = "239.0.0.1"
MCAST_PORT = 12_345
LOOPBACK_IF = "127.0.0.1"
BUF_SIZE = 1024
PHASE_WRAP_MIN_JUMP = 32_768

def get_current_time_ms():
    return int(time.monotonic() * 1000)


def phase_wrapped_across_zero(prev_phase, current_phase):
    return (
        prev_phase > current_phase
        and (prev_phase - current_phase) >= PHASE_WRAP_MIN_JUMP
    )

def create_multicast_socket(is_sender, mcast_group, mcast_port):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_LOOP, 1)

    loopback_if = socket.inet_aton(LOOPBACK_IF)

    if is_sender:
        # Use loopback multicast interface to interoperate with Rust sender/listener.
        sock.bind(("", 0))
        sock.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_TTL, 1)
        sock.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF, loopback_if)
    else:
        # Bind INADDR_ANY:port and join group on loopback interface.
        sock.bind(("", mcast_port))
        group = socket.inet_aton(mcast_group)
        mreq = struct.pack("4s4s", group, loopback_if)
        sock.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP, mreq)
        sock.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF, loopback_if)
        sock.settimeout(0.03)

    return sock

def sender_loop(sock, core, bpm_limit, mcast_group, mcast_port):
    print(f"Running as SENDER to {mcast_group}:{mcast_port}...")
    last_update_ms = get_current_time_ms()
    last_send_ms = 0
    last_phase = core.phase

    while True:
        now_ms = get_current_time_ms()
        dt_ms = now_ms - last_update_ms
        if dt_ms <= 0:
            time.sleep(0.001)
            continue
        last_update_ms = now_ms

        core.update(dt_ms, bpm_limit)

        # Send message every beat
        if core.phase < last_phase or (now_ms - last_send_ms > 1000):
            msg = core.to_message(now_ms)
            sock.sendto(msg.to_wire_bytes(), (mcast_group, mcast_port))
            last_send_ms = now_ms
            print(f"Sent: {msg} | BPM: {bpm_q8_to_float(core.current_bpm_raw):.2f}")

        last_phase = core.phase
        time.sleep(0.01)

def listener_loop(sock, core, bpm_limit, mcast_group, mcast_port):
    print(f"Running as LISTENER on {mcast_group}:{mcast_port}...")
    last_update_ms = get_current_time_ms()
    last_phase = None

    while True:
        now_ms = get_current_time_ms()
        dt_ms = now_ms - last_update_ms
        if dt_ms > 0:
            core.update(dt_ms, bpm_limit)
            last_update_ms = now_ms

        try:
            data, _ = sock.recvfrom(BUF_SIZE)
            if data:
                now_ms = get_current_time_ms()
                msg = RhythmMessage.from_wire_slice(data)
                core.sync(msg, now_ms, bpm_limit)

        except socket.timeout:
            pass # No message received
        
        should_print = (
            phase_wrapped_across_zero(last_phase, core.phase)
            if last_phase is not None
            else False
        )
        last_phase = core.phase

        if should_print:
            state_str = "Idle"
            if core.sync_state == SyncState.WaitSecondPoint:
                state_str = "Waiting"
            elif core.sync_state == SyncState.Locked:
                state_str = "Locked"

            print(
                f"[beat] t={now_ms}ms beat={core.beat_count} "
                f"phase={core.phase} bpm={bpm_q8_to_float(core.current_bpm_raw):.2f} "
                f"sync={state_str}"
            )
        


def main():
    parser = argparse.ArgumentParser(description="Python rhythm-core multicast example.")
    parser.add_argument("mode", choices=["listener", "sender"], help="Run as listener or sender")
    parser.add_argument("--group", default=MCAST_GROUP, help="Multicast group address")
    parser.add_argument("--port", type=int, default=MCAST_PORT, help="Multicast port")
    parser.add_argument("--bpm", type=int, default=120, help="Initial BPM")
    parser.add_argument("--k", type=int, default=12, help="Coupling divisor")
    args = parser.parse_args()

    is_sender = args.mode == "sender"
    
    # Use integer BPM values for limits
    bpm_limit = BpmLimitParam(60, 120)

    # Create RhythmGenerator with integer BPM.
    core = RhythmGenerator.from_int_bpm(0, args.bpm, args.k)
    
    sock = create_multicast_socket(is_sender, args.group, args.port)

    try:
        if is_sender:
            sender_loop(sock, core, bpm_limit, args.group, args.port)
        else:
            listener_loop(sock, core, bpm_limit, args.group, args.port)
    except KeyboardInterrupt:
        print("Exiting.")
    finally:
        sock.close()

if __name__ == "__main__":
    main()
