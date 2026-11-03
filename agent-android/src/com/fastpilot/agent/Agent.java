package com.fastpilot.agent;

import android.accessibilityservice.AccessibilityServiceInfo;
import android.app.Instrumentation;
import android.app.UiAutomation;
import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.Rect;
import android.os.Build;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import android.os.SystemClock;
import android.util.Base64;
import android.util.DisplayMetrics;
import android.view.InputDevice;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.WindowManager;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityWindowInfo;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.EOFException;
import java.io.InputStream;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.concurrent.TimeoutException;

/**
 * On-device half of the fastpilot Android driver. See PROTOCOL.md.
 * Start: adb shell am instrument -w -e port 7912 com.fastpilot.agent/.Agent
 */
public class Agent extends Instrumentation {
    static final String VERSION = "1";
    static final int MAX_FRAME = 16 << 20;

    private int port = 7912;
    private UiAutomation ui;
    private boolean quit;

    @Override
    public void onCreate(Bundle args) {
        super.onCreate(args);
        if (args != null && args.getString("port") != null) port = Integer.parseInt(args.getString("port"));
        start();
    }

    @Override
    public void onStart() {
        Bundle result = new Bundle();
        try {
            ui = getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES);
            AccessibilityServiceInfo info = ui.getServiceInfo();
            info.flags |= AccessibilityServiceInfo.FLAG_INCLUDE_NOT_IMPORTANT_VIEWS
                    | AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS
                    | AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS;
            ui.setServiceInfo(info);
            serve();
            result.putString("stream", "FASTPILOT_AGENT_EXIT\n");
        } catch (Throwable t) {
            result.putString("stream", "FASTPILOT_AGENT_ERROR " + t + "\n");
        }
        finish(0, result);
    }

    private void serve() throws Exception {
        ServerSocket server = new ServerSocket();
        server.setReuseAddress(true);
        server.bind(new java.net.InetSocketAddress(InetAddress.getByName("127.0.0.1"), port), 1);
        String ready = "FASTPILOT_AGENT_READY port=" + port + " version=" + VERSION;
        Bundle status = new Bundle();
        status.putString("stream", ready + "\n");
        sendStatus(1, status);
        System.out.println(ready);
        // ponytail: one client at a time, requests handled sequentially.
        while (!quit) {
            try (Socket s = server.accept()) {
                s.setTcpNoDelay(true);
                handleClient(s);
            } catch (Exception e) {
                System.out.println("fastpilot client error: " + e);
            }
        }
        server.close();
    }

    private void handleClient(Socket s) throws Exception {
        DataInputStream in = new DataInputStream(s.getInputStream());
        DataOutputStream out = new DataOutputStream(s.getOutputStream());
        while (!quit) {
            int len;
            try {
                len = in.readInt();
            } catch (EOFException e) {
                return; // client disconnected
            }
            if (len < 0 || len > MAX_FRAME) throw new IllegalStateException("bad frame length " + len);
            byte[] buf = new byte[len];
            in.readFully(buf);
            JSONObject resp = handle(new String(buf, StandardCharsets.UTF_8));
            byte[] o = resp.toString().getBytes(StandardCharsets.UTF_8);
            out.writeInt(o.length);
            out.write(o);
            out.flush();
        }
    }

    private JSONObject handle(String raw) {
        JSONObject resp = new JSONObject();
        try {
            JSONObject req = new JSONObject(raw);
            resp.put("id", req.opt("id"));
            dispatch(req, resp);
            resp.put("ok", true);
        } catch (Throwable t) {
            try {
                resp.put("ok", false);
                resp.put("error", t.getMessage() != null ? t.getMessage() : t.toString());
            } catch (Exception ignored) {
            }
        }
        return resp;
    }

    private void dispatch(JSONObject req, JSONObject r) throws Exception {
        String cmd = req.getString("cmd");
        switch (cmd) {
            case "hello": {
                DisplayMetrics m = new DisplayMetrics();
                WindowManager wm = (WindowManager) getContext().getSystemService(Context.WINDOW_SERVICE);
                wm.getDefaultDisplay().getRealMetrics(m);
                r.put("version", VERSION).put("sdk", Build.VERSION.SDK_INT)
                        .put("width", m.widthPixels).put("height", m.heightPixels).put("density", m.density);
                break;
            }
            case "snapshot":
                snapshot(r);
                break;
            case "tap": {
                float x = (float) req.getDouble("x"), y = (float) req.getDouble("y");
                long t = SystemClock.uptimeMillis();
                touch(t, t, MotionEvent.ACTION_DOWN, x, y);
                touch(t, SystemClock.uptimeMillis(), MotionEvent.ACTION_UP, x, y);
                break;
            }
            case "swipe": {
                float x1 = (float) req.getDouble("x1"), y1 = (float) req.getDouble("y1");
                float x2 = (float) req.getDouble("x2"), y2 = (float) req.getDouble("y2");
                long ms = req.optLong("ms", 300);
                int steps = 10;
                long t = SystemClock.uptimeMillis();
                touch(t, t, MotionEvent.ACTION_DOWN, x1, y1);
                for (int i = 1; i <= steps; i++) {
                    SystemClock.sleep(ms / steps);
                    float f = i / (float) steps;
                    touch(t, SystemClock.uptimeMillis(), MotionEvent.ACTION_MOVE, x1 + (x2 - x1) * f, y1 + (y2 - y1) * f);
                }
                touch(t, SystemClock.uptimeMillis(), MotionEvent.ACTION_UP, x2, y2);
                break;
            }
            case "text": {
                AccessibilityNodeInfo root = ui.getRootInActiveWindow();
                AccessibilityNodeInfo f = root == null ? null : root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT);
                if (f == null) throw new IllegalStateException("no focused text field");
                Bundle a = new Bundle();
                a.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, req.getString("text"));
                boolean ok = f.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, a);
                f.recycle();
                root.recycle();
                if (!ok) throw new IllegalStateException("set text failed");
                break;
            }
            case "key": {
                int code = req.getInt("code");
                long t = SystemClock.uptimeMillis();
                key(new KeyEvent(t, t, KeyEvent.ACTION_DOWN, code, 0));
                key(new KeyEvent(t, SystemClock.uptimeMillis(), KeyEvent.ACTION_UP, code, 0));
                break;
            }
            case "idle":
                try {
                    ui.waitForIdle(req.optLong("quietMs", 300), req.optLong("timeoutMs", 5000));
                    r.put("idle", true);
                } catch (TimeoutException e) {
                    r.put("idle", false);
                }
                break;
            case "screenshot": {
                Bitmap b = ui.takeScreenshot();
                if (b == null) throw new IllegalStateException("screenshot failed");
                ByteArrayOutputStream bo = new ByteArrayOutputStream();
                b.compress(Bitmap.CompressFormat.JPEG, req.optInt("quality", 80), bo);
                b.recycle();
                r.put("jpeg", Base64.encodeToString(bo.toByteArray(), Base64.NO_WRAP));
                break;
            }
            case "shell": {
                ParcelFileDescriptor pfd = ui.executeShellCommand(req.getString("command"));
                ByteArrayOutputStream bo = new ByteArrayOutputStream();
                try (InputStream is = new ParcelFileDescriptor.AutoCloseInputStream(pfd)) {
                    byte[] buf = new byte[8192];
                    for (int n; (n = is.read(buf)) > 0; ) bo.write(buf, 0, n);
                }
                r.put("out", new String(bo.toByteArray(), StandardCharsets.UTF_8));
                break;
            }
            case "quit":
                quit = true;
                break;
            default:
                throw new IllegalArgumentException("unknown cmd: " + cmd);
        }
    }

    private void touch(long down, long now, int action, float x, float y) throws InterruptedException {
        MotionEvent e = MotionEvent.obtain(down, now, action, x, y, 0);
        e.setSource(InputDevice.SOURCE_TOUCHSCREEN);
        // Only the final event waits for dispatch; earlier ones queue in order (halves tap latency).
        if (action == MotionEvent.ACTION_DOWN || action == MotionEvent.ACTION_MOVE) {
            boolean ok = ui.injectInputEvent(e, false);
            e.recycle();
            if (!ok) throw new IllegalStateException("inject motion failed");
        } else {
            injectBounded(e, "motion");
        }
    }

    private void key(KeyEvent e) throws InterruptedException {
        e.setSource(InputDevice.SOURCE_KEYBOARD);
        injectBounded(e, "key");
    }

    /**
     * Synchronous injection, but never blocks the agent for more than 3 s: a hung app (ANR in
     * the making) keeps the event queued while snapshots go on, so the ANR dialog can be seen.
     */
    private void injectBounded(android.view.InputEvent e, String what) throws InterruptedException {
        final boolean[] ok = {true};
        Thread t = new Thread(() -> ok[0] = ui.injectInputEvent(e, true));
        t.start();
        t.join(3000);
        if (t.isAlive()) return; // still queued behind a busy main thread
        if (e instanceof MotionEvent) ((MotionEvent) e).recycle();
        if (!ok[0]) throw new IllegalStateException("inject " + what + " failed");
    }

    private void snapshot(JSONObject r) throws Exception {
        JSONArray nodes = new JSONArray();
        String pkg = "";
        AccessibilityNodeInfo active = ui.getRootInActiveWindow();
        if (active != null && active.getPackageName() != null) pkg = active.getPackageName().toString();
        boolean any = false;
        String title = "";
        List<AccessibilityWindowInfo> windows = ui.getWindows();
        // getWindows() is ordered top-most first. Keep app windows and system dialogs
        // (e.g. ANR / "android" package alerts); skip status bar, nav bar and IME.
        for (AccessibilityWindowInfo w : windows) {
            int type = w.getType();
            // The soft keyboard: the host hides it when it covers a tap target.
            if (type == AccessibilityWindowInfo.TYPE_INPUT_METHOD) {
                w.getBoundsInScreen(RECT);
                r.put("ime", new JSONArray().put(RECT.left).put(RECT.top).put(RECT.right).put(RECT.bottom));
            }
            // Top-most app window title = activity label, used as the screen name.
            if (type == AccessibilityWindowInfo.TYPE_APPLICATION && title.isEmpty() && w.getTitle() != null) {
                title = w.getTitle().toString();
            }
            if (type == AccessibilityWindowInfo.TYPE_APPLICATION || type == AccessibilityWindowInfo.TYPE_SYSTEM) {
                AccessibilityNodeInfo root = w.getRoot();
                if (root != null) {
                    CharSequence p = root.getPackageName();
                    boolean systemUi = p != null && p.toString().equals("com.android.systemui");
                    if (type == AccessibilityWindowInfo.TYPE_APPLICATION || !systemUi) {
                        JSONObject n = node(root);
                        if (n != null) {
                            // Window owner: tells the host a permission prompt or crash/ANR
                            // dialog ("android") apart from the app under test.
                            if (p != null) n.put("pkg", p.toString());
                            nodes.put(n);
                            any = true;
                        }
                    }
                    root.recycle();
                }
            }
            w.recycle();
        }
        if (!any && active != null) {
            JSONObject n = node(active);
            if (n != null) nodes.put(n);
        }
        if (active != null) active.recycle();
        r.put("package", pkg).put("title", title).put("nodes", nodes);
    }

    private static final Rect RECT = new Rect();

    /** Serializes a visible node subtree; caller recycles {@code n}. */
    private JSONObject node(AccessibilityNodeInfo n) throws Exception {
        if (!n.isVisibleToUser()) return null;
        JSONObject o = new JSONObject();
        put(o, "cls", n.getClassName());
        boolean password = n.isPassword();
        // Never leak password contents; an empty field reports its hint as text, drop that too.
        boolean showingHint = Build.VERSION.SDK_INT >= 26 && n.isShowingHintText();
        if (!password && !showingHint) put(o, "text", n.getText());
        put(o, "desc", n.getContentDescription());
        if (Build.VERSION.SDK_INT >= 26) put(o, "hint", n.getHintText());
        put(o, "id", n.getViewIdResourceName());
        n.getBoundsInScreen(RECT);
        o.put("bounds", new JSONArray().put(RECT.left).put(RECT.top).put(RECT.right).put(RECT.bottom));
        flag(o, "clickable", n.isClickable());
        flag(o, "editable", n.isEditable());
        flag(o, "checkable", n.isCheckable());
        flag(o, "checked", n.isChecked());
        flag(o, "enabled", n.isEnabled());
        flag(o, "focused", n.isFocused());
        flag(o, "selected", n.isSelected());
        flag(o, "password", password);
        flag(o, "scrollable", n.isScrollable());
        int c = n.getChildCount();
        JSONArray kids = null;
        for (int i = 0; i < c; i++) {
            AccessibilityNodeInfo ch = n.getChild(i);
            if (ch == null) continue;
            JSONObject k = node(ch);
            ch.recycle();
            if (k == null) continue;
            if (kids == null) kids = new JSONArray();
            kids.put(k);
        }
        if (kids != null) o.put("children", kids);
        return o;
    }

    private static void put(JSONObject o, String k, CharSequence v) throws Exception {
        if (v != null && v.length() > 0) o.put(k, v.toString());
    }

    private static void flag(JSONObject o, String k, boolean v) throws Exception {
        if (v) o.put(k, true);
    }
}
