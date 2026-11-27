// Keep on Top for Herb's Download Manager on GNOME (§3 T11). GNOME on Wayland lets no app keep its own windows
// above others, so the app installs this helper and asks it over the session bus: SetAbove(app_id, on) keeps every
// window of the app with that GTK application id above other windows, the ones it opens later too; IsAbove(app_id)
// says whether they are. The app id
// tells a test copy (org.herbsdownloadmanager.app.Test…) from the user's own copy.
import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const BUS_NAME = 'org.herbsdownloadmanager.KeepOnTop';
const OBJECT_PATH = '/org/herbsdownloadmanager/KeepOnTop';
const INTERFACE = `<node>
  <interface name="org.herbsdownloadmanager.KeepOnTop">
    <method name="SetAbove">
      <arg type="s" direction="in" name="app_id"/>
      <arg type="b" direction="in" name="on"/>
    </method>
    <method name="IsAbove">
      <arg type="s" direction="in" name="app_id"/>
      <arg type="b" direction="out" name="above"/>
    </method>
    <property name="Version" type="u" access="read"/>
  </interface>
</node>`;

export default class KeepOnTop extends Extension {
    enable() {
        this._apps = new Set();
        this._watched = new Map();
        this._object = Gio.DBusExportedObject.wrapJSObject(INTERFACE, this);
        this._object.export(Gio.DBus.session, OBJECT_PATH);
        this._name = Gio.bus_own_name_on_connection(Gio.DBus.session, BUS_NAME, Gio.BusNameOwnerFlags.NONE, null, null);
        this._created = global.display.connect('window-created', (_display, window) => this._watch(window));
    }

    disable() {
        global.display.disconnect(this._created);
        Gio.bus_unown_name(this._name);
        this._object.unexport();
        for (const [window, id] of this._watched)
            window.disconnect(id);
        for (const window of this._windows()) {
            if (this._apps.has(this._appId(window)))
                window.unmake_above();
        }
        this._watched = null;
        this._apps = null;
        this._object = null;
    }

    get Version() {
        return 1;
    }

    SetAbove(appId, on) {
        if (on)
            this._apps.add(appId);
        else
            this._apps.delete(appId);
        for (const window of this._windows()) {
            if (this._appId(window) === appId)
                this._place(window);
        }
    }

    // Whether the app has windows and every one is kept above (the app's tests ask).
    IsAbove(appId) {
        const windows = this._windows().filter(window => this._appId(window) === appId);
        return windows.length > 0 && windows.every(window => window.is_above());
    }

    _windows() {
        return global.get_window_actors().map(actor => actor.get_meta_window()).filter(window => window);
    }

    _appId(window) {
        return window.get_gtk_application_id() ?? '';
    }

    _place(window) {
        const above = this._apps.has(this._appId(window));
        if (above && !window.is_above())
            window.make_above();
        else if (!above && window.is_above())
            window.unmake_above();
    }

    // A new window may get its application id just after it is made: place it then, and once it's shown.
    _watch(window) {
        const id = window.connect('notify::gtk-application-id', () => this._apply(window));
        this._watched.set(window, id);
        window.connect('unmanaged', () => {
            this._watched?.get(window) && window.disconnect(this._watched.get(window));
            this._watched?.delete(window);
        });
        GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
            this._apply(window);
            return GLib.SOURCE_REMOVE;
        });
    }

    _apply(window) {
        if (this._apps && this._apps.has(this._appId(window)))
            window.make_above();
    }
}
