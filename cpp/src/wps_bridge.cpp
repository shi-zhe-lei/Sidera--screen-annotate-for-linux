// Sidera - shared WPS bridge for the legacy X11 and Wayland frontends.
// Copyright (C) 2026 Carl_Jin   GNU GPL v3
#include "wps_bridge.h"
#include "bridge_http.h"
#include <QTcpServer>
#include <QHostAddress>
#include <QTimer>
#include <QCoreApplication>
#include <QStandardPaths>
#include <QProcess>
#include <QDateTime>
#include <QDebug>
#include <QRegularExpression>

WpsBridge::WpsBridge(QObject* parent) : QObject(parent) { clock_.start(); }
WpsBridge::~WpsBridge() { stop(); }
quint16 WpsBridge::port() const { return server_ ? server_->serverPort() : 0; }
QString WpsBridge::logFile() const {
  return QDir::homePath() + "/wps-api-debug.log";
}
void WpsBridge::log(const QString& message) {
  qDebug().noquote() << "[WPSAPI]" << message;
  const QString path = logFile();
  if (QFileInfo(path).isSymLink()) return;
  QFile file(path);
  const auto mode = QFileInfo(path).size() > 1024 * 1024 ? QIODevice::Truncate : QIODevice::Append;
  if (file.open(QIODevice::WriteOnly | QIODevice::Text | mode))
    file.write((QDateTime::currentDateTime().toString(Qt::ISODate) + " " + message + "\n").toUtf8());
}
QString WpsBridge::addinDir() const {
  if (qEnvironmentVariableIsSet("WPS_ADDIN_DIR")) {
    const QString directory = QString::fromLocal8Bit(qgetenv("WPS_ADDIN_DIR"));
    return QFile::exists(directory + "/manifest.xml") ? directory : QString();
  }
  for (const auto& path : {QCoreApplication::applicationDirPath() + "/wps-addin", QString("/usr/share/sidera/wps-addin")})
    if (!path.isEmpty() && QFile::exists(path + "/manifest.xml")) return path;
  return {};
}
void WpsBridge::ensureAddinRegistered() {
  const QString directory = addinDir();
  if (directory.isEmpty() || !QFile::exists(directory + "/install.sh")) return;
  QProcess process;
  process.start("bash", {directory + "/install.sh"});
  if (!process.waitForStarted(1000) || !process.waitForFinished(5000)) {
    process.kill(); process.waitForFinished(1000); log("加载项登记超时"); return;
  }
  if (process.exitStatus() != QProcess::NormalExit || process.exitCode() != 0)
    log("加载项登记失败：" + QString::fromUtf8(process.readAllStandardError()).left(2048));
}
bool WpsBridge::start(quint16 port, bool registerAddin) {
  if (server_) return true;
  server_ = new QTcpServer(this);
  server_->setMaxPendingConnections(16);
  connect(server_, &QTcpServer::newConnection, this, [this]() { onNewConnection(); });
  if (!server_->listen(QHostAddress::LocalHost, port)) {
    log("HTTP 绑定失败：" + server_->errorString());
    server_->deleteLater(); server_ = nullptr; return false;
  }
  if (registerAddin) ensureAddinRegistered();
  ping_ = new QTimer(this);
  connect(ping_, &QTimer::timeout, this, [this]() {
    if (connected_ && clock_.elapsed() - lastSeen_ > 3000) {
      connected_ = false; queue_.clear();
      // Keep the last position: reconnecting to the same page must not clear ink.
      if (onConnectionChanged) onConnectionChanged(false);
    }
  });
  ping_->start(1000);
  return true;
}
void WpsBridge::stop() {
  if (ping_) { ping_->stop(); ping_->deleteLater(); ping_ = nullptr; }
  if (server_) {
    server_->close(); disconnect(server_, nullptr, this, nullptr);
    for (auto* socket : server_->findChildren<QTcpSocket*>()) {
      disconnect(socket, nullptr, this, nullptr); socket->abort(); socket->deleteLater();
    }
    server_->deleteLater(); server_ = nullptr;
  }
  const bool wasConnected = connected_;
  connected_ = false; lastSeen_ = 0; realPos_ = -1; queue_.clear();
  missingPosition_.invalidate();
  deck_ = -1;
  sequences_.clear();
  if (wasConnected && onConnectionChanged) onConnectionChanged(false);
}
bool WpsBridge::enqueue(const QString& command) {
  if (!connected_ || realPos_ < 1 || whiteboard_ || clock_.elapsed() - lastSeen_ > 3000 ||
      !QStringList({"NEXT", "PREV", "EXIT"}).contains(command)) return false;
  while (!queue_.isEmpty() && clock_.elapsed() - queue_.head().first > 1500) queue_.dequeue();
  if (queue_.size() >= 8) return false;
  queue_.enqueue(qMakePair(clock_.elapsed(), command)); return true;
}
void WpsBridge::onNewConnection() {
  while (server_ && server_->hasPendingConnections()) {
    auto* socket = server_->nextPendingConnection();
    if (server_->findChildren<QTcpSocket*>().size() > 32) { socket->abort(); socket->deleteLater(); continue; }
    BridgeHttp::prepare(socket);
    connect(socket, &QTcpSocket::readyRead, this, [this, socket]() { handleHttp(socket); });
    if (socket->bytesAvailable()) handleHttp(socket);
  }
}
void WpsBridge::touch() {
  lastSeen_ = clock_.elapsed();
  if (!connected_) { connected_ = true; if (onConnectionChanged) onConnectionChanged(true); }
}
void WpsBridge::httpReply(QTcpSocket* socket, const QString& body) {
  BridgeHttp::reply(socket, 200, body.toUtf8());
}
void WpsBridge::serveAddinFile(QTcpSocket* socket, const QString& path) {
  BridgeHttp::serve(socket, addinDir(), path);
}
void WpsBridge::handleHttp(QTcpSocket* socket) {
  BridgeHttp::Request request;
  if (!BridgeHttp::read(socket, request)) return;
  const auto& path = request.path;
  const auto& message = request.message;
  if (request.method == "OPTIONS") {
    if (path == "/hello" || path == "/push" || path == "/poll" || BridgeHttp::staticPath(path)) httpReply(socket, "");
    else BridgeHttp::reply(socket, 404, "Not found");
    return;
  }
  if ((path == "/push" || path == "/poll") && (!request.session.isEmpty() || !request.sequence.isEmpty())) {
    bool valid = false;
    const quint64 sequence = request.sequence.toULongLong(&valid);
    if (!valid || !sequence || !QRegularExpression("^[a-zA-Z0-9-]{1,64}$").match(request.session).hasMatch()) {
      BridgeHttp::reply(socket, 400, "Invalid sequence"); return;
    }
    if (sequences_.contains(request.session) && sequence <= sequences_.value(request.session)) {
      httpReply(socket, path == "/push" ? "OK" : ""); return;
    }
    if (!sequences_.contains(request.session) && sequences_.size() >= 16) sequences_.remove(sequences_.firstKey());
    sequences_.insert(request.session, sequence);
  }
  if (path == "/hello") { httpReply(socket, "OK sidera"); return; }
  if (path == "/push") { handleLine(message); httpReply(socket, "OK"); return; }
  if (path == "/poll") {
    if (!message.startsWith("EVENT SlideShowState pos=")) { httpReply(socket, ""); return; }
    handleLine(message); touch();
    while (!queue_.isEmpty() && clock_.elapsed() - queue_.head().first > 1500) queue_.dequeue();
    if (whiteboard_ || realPos_ < 1) queue_.clear();
    httpReply(socket, queue_.isEmpty() ? QString() : queue_.dequeue().second);
    return;
  }
  serveAddinFile(socket, path);
}
void WpsBridge::handleLine(const QString& raw) {
  const QString line = raw.trimmed();
  if (line.startsWith("COMMAND_FAILED ")) { queue_.clear(); return; }
  if (!line.startsWith("EVENT ")) return;
  const QString name = line.section(' ', 1, 1);
  if (!QStringList({"SlideShowState", "SlideShowBegin", "SlideShowEnd", "SlideShowNextSlide",
      "SlideShowNextClick", "SlideShowOnNext", "SlideShowOnPrevious"}).contains(name)) return;
  const QRegularExpression expression("(?:^| )pos=(-?[0-9]+)(?: |$)");
  const auto match = expression.match(line);
  bool valid = false; const int position = match.captured(1).toInt(&valid);
  if (!valid) return;
  const auto deckMatch = QRegularExpression("(?:^| )deck=([0-9]+)(?: |$)").match(line);
  const int deck = deckMatch.hasMatch() ? deckMatch.captured(1).toInt() : -1;
  if (name == "SlideShowState" && position < 1 && realPos_ > 0) {
    // WPS can briefly lose the current slide during a transition.
    if (!missingPosition_.isValid()) missingPosition_.start();
    if (missingPosition_.elapsed() < 1200) return;
  }
  if (position > 0 || name == "SlideShowEnd") missingPosition_.invalidate();
  const bool changedDeck = name == "SlideShowState" && position > 0 && realPos_ > 0 && deck > 0 && deck_ > 0 && deck != deck_;
  if (name == "SlideShowState") deck_ = deck;
  if (name == "SlideShowEnd" || (name == "SlideShowState" && position < 1)) {
    const bool showing = realPos_ > 0; realPos_ = -1; queue_.clear();
    if (showing && onSlideshowEnd) onSlideshowEnd();
    return;
  }
  if (position > 0 && (position != realPos_ || changedDeck)) {
    const bool begin = realPos_ < 1 || changedDeck; realPos_ = position;
    if (changedDeck) queue_.clear();
    if (begin && onSlideshowBegin) onSlideshowBegin(position);
    else if (onRealPos) onRealPos(position);
  }
}
