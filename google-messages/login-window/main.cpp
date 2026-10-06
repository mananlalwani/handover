#include <QApplication>
#include <QLabel>
#include <QPushButton>
#include <QTimer>
#include <QVBoxLayout>
#include <QWebEngineDownloadRequest>
#include <QWebEnginePage>
#include <QWebEnginePermission>
#include <QWebEngineProfile>
#include <QWebEngineSettings>
#include <QWebEngineView>

class LoginPage final : public QWebEnginePage {
public:
    using QWebEnginePage::QWebEnginePage;

protected:
    bool acceptNavigationRequest(const QUrl &url, NavigationType, bool mainFrame) override
    {
        if (!mainFrame)
            return url.scheme() == QStringLiteral("https") || url == QUrl("about:blank");
        return url == QUrl("about:blank")
            || (url.scheme() == QStringLiteral("https")
                && (url.host() == QStringLiteral("accounts.google.com")
                    || url.host() == QStringLiteral("messages.google.com")));
    }
    void javaScriptConsoleMessage(JavaScriptConsoleMessageLevel, const QString &, int,
                                  const QString &) override {}
};

int main(int argc, char **argv)
{
    QApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("Handover Google sign-in probe"));
    const auto args = app.arguments();
    if (args.size() > 2 || (args.size() == 2 && args[1] != QStringLiteral("--self-test")))
        return 2;
    const bool selfTest = args.contains(QStringLiteral("--self-test"));

    // An unnamed profile is off the record. Do not use the shared default profile.
    QWebEngineProfile profile;
    profile.setHttpCacheType(QWebEngineProfile::MemoryHttpCache);
    profile.setPersistentCookiesPolicy(QWebEngineProfile::NoPersistentCookies);
    QObject::connect(&profile, &QWebEngineProfile::downloadRequested,
                     [](QWebEngineDownloadRequest *request) { request->cancel(); });

    QWidget window;
    window.setWindowTitle(QStringLiteral("Handover: Google Messages sign-in"));
    window.resize(900, 720);
    auto *layout = new QVBoxLayout(&window);
    auto *notice = new QLabel(QStringLiteral(
        "Sign-in compatibility test. This does not pair your phone or connect Handover.\n"
        "Close this window after sign-in, or if Google refuses it."));
    notice->setWordWrap(true);
    layout->addWidget(notice);
    auto *origin = new QLabel;
    layout->addWidget(origin);
    auto *view = new QWebEngineView;
    auto *page = new LoginPage(&profile, view);
    view->setPage(page);
    page->settings()->setAttribute(QWebEngineSettings::LocalContentCanAccessFileUrls, false);
    page->settings()->setAttribute(QWebEngineSettings::LocalContentCanAccessRemoteUrls, false);
    page->settings()->setAttribute(QWebEngineSettings::JavascriptCanOpenWindows, false);
    QObject::connect(page, &QWebEnginePage::permissionRequested,
                     [](const QWebEnginePermission &permission) { permission.deny(); });
    QObject::connect(page, &QWebEnginePage::urlChanged, origin, [origin](const QUrl &url) {
        // Display only the origin, never account-bearing paths or query strings.
        origin->setText(url.scheme() + QStringLiteral("://") + url.host());
    });
    layout->addWidget(view);
    auto *cancel = new QPushButton(QStringLiteral("Close"));
    QObject::connect(cancel, &QPushButton::clicked, &window, &QWidget::close);
    layout->addWidget(cancel);
    QTimer::singleShot(10 * 60 * 1000, &app, &QApplication::quit);

    if (selfTest) {
        if (!profile.isOffTheRecord()
            || profile.persistentCookiesPolicy() != QWebEngineProfile::NoPersistentCookies)
            return 1;
        QObject::connect(page, &QWebEnginePage::loadFinished, &app, [&app](bool ok) {
            app.exit(ok ? 0 : 1);
        });
        QTimer::singleShot(15000, &app, [&app] { app.exit(1); });
        view->setUrl(QUrl(QStringLiteral("about:blank")));
    } else {
        view->setUrl(QUrl(QStringLiteral(
            "https://accounts.google.com/AccountChooser?continue=https%3A%2F%2Fmessages.google.com%2Fweb%2Fconfig")));
        window.show();
    }
    return app.exec();
}
