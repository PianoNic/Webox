#!/usr/bin/env python3
"""A stand-in desktop on the session bus for the headless Linux test: a StatusNotifierWatcher (what a panel provides), a
notification server, and a listener for the launcher badge. It records what WebOx exports and drives it back:
reads the tray item's properties and menu, clicks the "Info" menu row, and answers each notification with the default action.
Every observation is printed as one JSON line.  Usage: desktop-mock.py  (runs until killed)"""
import json
from gi.repository import Gio, GLib

def log(**k):
    print(json.dumps(k, default=str), flush=True)

WATCHER_XML = """<node><interface name='org.kde.StatusNotifierWatcher'>
<method name='RegisterStatusNotifierItem'><arg type='s' direction='in'/></method>
<method name='RegisterStatusNotifierHost'><arg type='s' direction='in'/></method>
<property name='RegisteredStatusNotifierItems' type='as' access='read'/>
<property name='IsStatusNotifierHostRegistered' type='b' access='read'/>
<property name='ProtocolVersion' type='i' access='read'/>
</interface></node>"""
NOTIF_XML = """<node><interface name='org.freedesktop.Notifications'>
<method name='Notify'><arg type='s' direction='in'/><arg type='u' direction='in'/><arg type='s' direction='in'/><arg type='s' direction='in'/>
<arg type='s' direction='in'/><arg type='as' direction='in'/><arg type='a{sv}' direction='in'/><arg type='i' direction='in'/><arg type='u' direction='out'/></method>
<method name='CloseNotification'><arg type='u' direction='in'/></method>
<method name='GetCapabilities'><arg type='as' direction='out'/></method>
<method name='GetServerInformation'><arg type='s' direction='out'/><arg type='s' direction='out'/><arg type='s' direction='out'/><arg type='s' direction='out'/></method>
<signal name='ActionInvoked'><arg type='u'/><arg type='s'/></signal>
<signal name='NotificationClosed'><arg type='u'/><arg type='u'/></signal>
</interface></node>"""

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
items = []
next_id = [0]
clicked = [False]

def call(dest, path, iface, method, args):
    return bus.call_sync(dest, path, iface, method, args, None, Gio.DBusCallFlags.NONE, 3000, None).unpack()

def probe(name):
    try:
        p = call(name, "/StatusNotifierItem", "org.freedesktop.DBus.Properties", "GetAll", GLib.Variant("(s)", ("org.kde.StatusNotifierItem",)))[0]
        log(event="sni-props", id=p["Id"], title=p["Title"], status=p["Status"], category=p["Category"], menu=p["Menu"],
            tooltip=p["ToolTip"][2:], pixmaps=[(w, h, len(d)) for (w, h, d) in p["IconPixmap"]])
        rev, (rid, rprops, children) = call(name, p["Menu"], "com.canonical.dbusmenu", "GetLayout", GLib.Variant("(iias)", (0, -1, [])))
        rows = [(c[0], c[1].get("label"), c[1].get("enabled"), c[1].get("type", "standard")) for c in children]
        log(event="menu", revision=rev, rows=rows)
        if not clicked[0]:
            info = next(r for r in rows if r[1] == "Info")
            call(name, p["Menu"], "com.canonical.dbusmenu", "Event", GLib.Variant("(isvu)", (info[0], "clicked", GLib.Variant("s", ""), 0)))
            clicked[0] = True
            log(event="menu-clicked", row=info[1])
        call(name, "/StatusNotifierItem", "org.kde.StatusNotifierItem", "Activate", GLib.Variant("(ii)", (0, 0)))
        log(event="activated")
    except Exception as e:  # noqa: BLE001 - report and keep the mock alive
        log(event="probe-error", error=str(e))
    return False

def watcher_call(conn, sender, path, iface, method, params, inv):
    if method == "RegisterStatusNotifierItem":
        svc = params.unpack()[0]
        name = svc if not svc.startswith("/") else sender
        items.append(name)
        log(event="sni-registered", service=svc)
        GLib.timeout_add(1500, probe, name)
    inv.return_value(None)

def watcher_prop(conn, sender, path, iface, prop):
    return {"RegisteredStatusNotifierItems": GLib.Variant("as", items), "IsStatusNotifierHostRegistered": GLib.Variant("b", True),
            "ProtocolVersion": GLib.Variant("i", 0)}[prop]

def notif_call(conn, sender, path, iface, method, params, inv):
    if method == "Notify":
        app, rep, icon, summary, body, actions, hints, timeout = params.unpack()
        next_id[0] += 1
        nid = next_id[0]
        log(event="notify", id=nid, app=app, icon=icon, summary=summary, body=body, actions=actions, hints=hints)
        inv.return_value(GLib.Variant("(u)", (nid,)))
        def act():
            bus.emit_signal(None, "/org/freedesktop/Notifications", "org.freedesktop.Notifications", "ActionInvoked", GLib.Variant("(us)", (nid, "default")))
            log(event="action-invoked", id=nid, action="default")
            return False
        GLib.timeout_add(700, act)
    elif method == "CloseNotification":
        log(event="close-notification", id=params.unpack()[0]); inv.return_value(None)
    elif method == "GetCapabilities":
        inv.return_value(GLib.Variant("(as)", (["actions", "body"],)))
    else:
        inv.return_value(GLib.Variant("(ssss)", ("mock", "test", "1", "1.2")))

def launcher(conn, sender, path, iface, signal, params):
    uri, props = params.unpack()
    log(event="launcher-update", uri=uri, props=props)

for xml, path, name, fn, prop in [(WATCHER_XML, "/StatusNotifierWatcher", "org.kde.StatusNotifierWatcher", watcher_call, watcher_prop),
                                  (NOTIF_XML, "/org/freedesktop/Notifications", "org.freedesktop.Notifications", notif_call, None)]:
    info = Gio.DBusNodeInfo.new_for_xml(xml).interfaces[0]
    bus.register_object(path, info, fn, prop, None)
    Gio.bus_own_name_on_connection(bus, name, Gio.BusNameOwnerFlags.NONE, None, None)
bus.signal_subscribe(None, "com.canonical.Unity.LauncherEntry", "Update", None, None, Gio.DBusSignalFlags.NONE, launcher)
log(event="mock-ready")
GLib.MainLoop().run()
