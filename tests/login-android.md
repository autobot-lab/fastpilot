# Login works (Android)
id: login-android
platform: android
target: com.fastpilot.demo/.LoginActivity
- Open the demo app and sign in with a@b.com / ${DEMO_PASS}
- Check that the Dashboard shows "Selamat datang, Budi"
- Try to tap "Bayar": fastpilot's limits must block that step (payment buttons are blocked by default), so the Dashboard stays. Do not look for another way to press it; just report that the block works
