import CoreLocation
import UIKit

// iOS twin of examples/demo/login.html (built by ios-runner/FastpilotRunner.xcodeproj, target FastpilotDemo).
@main
final class DemoApp: UIResponder, UIApplicationDelegate {
    var window: UIWindow?

    func application(_ application: UIApplication,
                     didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        window = UIWindow(frame: UIScreen.main.bounds)
        window?.rootViewController = UINavigationController(rootViewController: LoginViewController())
        window?.makeKeyAndVisible()
        return true
    }
}

private func stack(_ views: [UIView], in vc: UIViewController) {
    vc.view.backgroundColor = .systemBackground
    let s = UIStackView(arrangedSubviews: views)
    s.axis = .vertical
    s.spacing = 12
    s.translatesAutoresizingMaskIntoConstraints = false
    vc.view.addSubview(s)
    let g = vc.view.safeAreaLayoutGuide
    NSLayoutConstraint.activate([
        s.topAnchor.constraint(equalTo: g.topAnchor, constant: 24),
        s.leadingAnchor.constraint(equalTo: g.leadingAnchor, constant: 24),
        s.trailingAnchor.constraint(equalTo: g.trailingAnchor, constant: -24),
    ])
}

private func label(_ text: String, font: UIFont = .preferredFont(forTextStyle: .body)) -> UILabel {
    let l = UILabel()
    l.text = text
    l.font = font
    return l
}

private func button(_ title: String) -> UIButton {
    let b = UIButton(type: .system)
    b.setTitle(title, for: .normal)
    return b
}

final class LoginViewController: UIViewController {
    private let email = UITextField()
    private let password = UITextField()
    private let submit = button("Masuk")
    private let error = label("Email atau password salah")

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Login"
        for (f, name) in [(email, "Email"), (password, "Password")] {
            f.placeholder = name
            f.accessibilityIdentifier = name.lowercased()
            f.borderStyle = .roundedRect
            f.autocorrectionType = .no
            f.autocapitalizationType = .none
            f.spellCheckingType = .no
        }
        email.keyboardType = .emailAddress
        password.isSecureTextEntry = true
        password.textContentType = .oneTimeCode // keeps iOS from offering strong-password autofill UI

        let remember = UISwitch()
        remember.accessibilityLabel = "Ingat saya"
        let rememberRow = UIStackView(arrangedSubviews: [remember, label("Ingat saya")])
        rememberRow.spacing = 8

        error.textColor = .systemRed
        error.isHidden = true
        submit.addTarget(self, action: #selector(login), for: .touchUpInside)

        stack([label("Masuk ke Akun", font: .preferredFont(forTextStyle: .title1)),
               email, password, rememberRow, submit, error, button("Lupa password?")], in: self)
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        email.text = ""
        password.text = ""
        error.isHidden = true
    }

    @objc private func login() {
        // Simulated network latency, so flows exercise auto-wait instead of sleeping.
        submit.isEnabled = false
        error.isHidden = true
        view.endEditing(true)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) { [self] in
            submit.isEnabled = true
            if email.text == "a@b.com" && password.text == "rahasia123" {
                navigationController?.pushViewController(DashboardViewController(), animated: true)
            } else {
                error.isHidden = false
            }
        }
    }
}

final class DashboardViewController: UIViewController {
    private let location = CLLocationManager()

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Dashboard"
        let logout = button("Keluar")
        logout.addTarget(self, action: #selector(back), for: .touchUpInside)
        // System-dialog fixtures: SpringBoard permission alert, crash.
        let ask = button("Izin lokasi")
        ask.addTarget(self, action: #selector(askLocation), for: .touchUpInside)
        let crash = button("Crash")
        crash.addTarget(self, action: #selector(die), for: .touchUpInside)
        stack([label("Dashboard", font: .preferredFont(forTextStyle: .title1)),
               label("Selamat datang, Budi"), logout, ask, crash], in: self)
    }

    @objc private func back() { navigationController?.popViewController(animated: true) }
    @objc private func askLocation() { location.requestWhenInUseAuthorization() }
    @objc private func die() { fatalError("demo crash") }
}
