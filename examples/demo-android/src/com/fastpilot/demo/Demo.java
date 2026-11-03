package com.fastpilot.demo;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.text.InputType;
import android.view.View;
import android.webkit.WebView;
import android.widget.Button;
import android.widget.CheckBox;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.TextView;

/** Android twin of examples/demo/login.html. Views are built in code; no XML layouts. */
public final class Demo {
    private Demo() {}

    static LinearLayout column(Activity a) {
        LinearLayout l = new LinearLayout(a);
        l.setOrientation(LinearLayout.VERTICAL);
        int p = (int) (24 * a.getResources().getDisplayMetrics().density);
        l.setPadding(p, p, p, p);
        l.setFitsSystemWindows(true);
        return l;
    }

    static TextView text(Activity a, String s, float sp) {
        TextView t = new TextView(a);
        t.setText(s);
        t.setTextSize(sp);
        return t;
    }

    static Button button(Activity a, String s) {
        Button b = new Button(a);
        b.setText(s);
        b.setAllCaps(false); // keep a11y text identical to the label ("Masuk", not "MASUK")
        return b;
    }

    public static class LoginActivity extends Activity {
        @Override
        protected void onCreate(Bundle b) {
            super.onCreate(b);
            setTitle("Login");
            LinearLayout l = column(this);
            l.addView(text(this, "Masuk ke Akun", 24));
            EditText email = new EditText(this);
            email.setHint("Email");
            email.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS);
            l.addView(email);
            EditText password = new EditText(this);
            password.setHint("Password");
            password.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD);
            l.addView(password);
            CheckBox remember = new CheckBox(this);
            remember.setText("Ingat saya");
            l.addView(remember);
            Button submit = button(this, "Masuk");
            l.addView(submit);
            TextView error = text(this, "", 16);
            error.setTextColor(0xFFB00020);
            error.setVisibility(View.GONE);
            l.addView(error);
            TextView forgot = text(this, "Lupa password?", 16);
            forgot.setOnClickListener(v -> {});
            l.addView(forgot);
            // Simulated network latency, so flows exercise auto-wait instead of sleeping.
            submit.setOnClickListener(v -> {
                submit.setEnabled(false);
                error.setVisibility(View.GONE);
                new Handler(Looper.getMainLooper()).postDelayed(() -> {
                    submit.setEnabled(true);
                    if (email.getText().toString().equals("a@b.com")
                            && password.getText().toString().equals("rahasia123")) {
                        startActivity(new Intent(this, DashboardActivity.class));
                    } else {
                        error.setText("Email atau password salah");
                        error.setVisibility(View.VISIBLE);
                    }
                }, 400);
            });
            setContentView(l);
        }
    }

    public static class DashboardActivity extends Activity {
        @Override
        protected void onCreate(Bundle b) {
            super.onCreate(b);
            setTitle("Dashboard");
            LinearLayout l = column(this);
            l.addView(text(this, "Dashboard", 24));
            l.addView(text(this, "Selamat datang, Budi", 16));
            Button pay = button(this, "Bayar");
            pay.setOnClickListener(v -> startActivity(new Intent(this, PayActivity.class)));
            l.addView(pay);
            Button logout = button(this, "Keluar");
            logout.setOnClickListener(v -> finish());
            l.addView(logout);
            setContentView(l);
        }
    }

    static final String PAY_HTML = "<!doctype html><html><head><meta charset='utf-8'>"
            + "<meta name='viewport' content='width=device-width'><title>Pembayaran</title></head>"
            + "<body style='font-family:sans-serif'><main id='pay'><h1>Pembayaran</h1><p>Total Rp 150.000</p>"
            + "<button id='paynow' onclick=\"this.disabled=true;setTimeout(function(){"
            + "document.getElementById('pay').innerHTML='<h1>Pembayaran berhasil</h1>'},300)\">Bayar sekarang</button>"
            + "</main></body></html>";

    public static class PayActivity extends Activity {
        @Override
        protected void onCreate(Bundle b) {
            super.onCreate(b);
            setTitle("Pembayaran");
            WebView.setWebContentsDebuggingEnabled(true);
            WebView w = new WebView(this);
            w.setFitsSystemWindows(true);
            w.getSettings().setJavaScriptEnabled(true);
            w.loadDataWithBaseURL("https://demo.fastpilot.local/", PAY_HTML, "text/html", "utf-8", null);
            setContentView(w);
        }
    }
}
