#!/usr/bin/env python3
"""Own a private test Secret Service and deliberately leave OpenSession unanswered."""
from gi.repository import Gio, GLib

XML = """<node><interface name="org.freedesktop.Secret.Service">
<method name="OpenSession"><arg type="s" direction="in"/>
<arg type="v" direction="in"/><arg type="v" direction="out"/>
<arg type="o" direction="out"/></method>
<method name="SearchItems"><arg type="a{ss}" direction="in"/>
<arg type="ao" direction="out"/><arg type="ao" direction="out"/></method>
<method name="ReadAlias"><arg type="s" direction="in"/><arg type="o" direction="out"/></method>
<property name="Collections" type="ao" access="read"/>
</interface></node>"""
held = []


def acquired(connection, _):
    def method(conn, sender, path, interface, name, parameters, invocation):
        held.append(invocation)
        print("Secret Service received " + name + "; deliberately no reply", flush=True)

    def prop(*_):
        return GLib.Variant("ao", [])

    connection.register_object("/org/freedesktop/secrets",
                               Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0],
                               method, prop, None)



Gio.bus_own_name(Gio.BusType.SESSION, "org.freedesktop.secrets",
                 Gio.BusNameOwnerFlags.NONE, acquired, lambda *_: print("test Secret Service ready", flush=True), None)
GLib.MainLoop().run()
