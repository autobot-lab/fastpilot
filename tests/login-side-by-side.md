# Login on web, Android and iPhone
id: login-side-by-side
platform: web, android, ios
target web: examples/demo/login.html
target android: com.fastpilot.demo/.LoginActivity
target ios: com.fastpilot.demo
- Open the login screen and check that "Masuk ke Akun" shows
- Check that there is a link for a forgotten password, and note its exact text
- Sign in with a@b.com / ${DEMO_PASS} and tap "Masuk"
- Check that the Dashboard shows "Selamat datang, Budi"
