#pragma once
// ============================================================
// Sidera - WPS 联动桥（平台无关：本地 HTTP 16666 + 加载项注册 + 心跳）
// 与 X11/Wayland 无关，纯 TCP + 事件回调。Wayland 后端使用。
// Copyright (C) 2026 Carl_Jin   GNU GPL v3
// ============================================================
#include <QObject>
#include <QString>
#include <QQueue>
#include <QElapsedTimer>
#include <QMap>
#include <functional>

class QTcpServer;
class QTcpSocket;
class QTimer;

class WpsBridge : public QObject {
public:
  explicit WpsBridge(QObject* parent = nullptr);
  ~WpsBridge() override;

  bool start(quint16 port = 16666, bool registerAddin = true);
  void stop();
  bool enqueue(const QString& cmd);  // NEXT / PREV / EXIT, bounded and expiring
  void setWhiteboard(bool enabled) { whiteboard_ = enabled; if (enabled) queue_.clear(); }
  int position() const { return realPos_; }
  quint16 port() const;
  bool connected() const { return connected_; }
  QString logFile() const;

  // 回调（由使用者设置）
  std::function<void(int)> onRealPos;        // 真实页号变化（去重后）
  std::function<void(int)> onSlideshowBegin; // 放映开始
  std::function<void()> onSlideshowEnd;
  std::function<void(bool)> onConnectionChanged;

private:
  void onNewConnection();
  void handleHttp(QTcpSocket* s);
  void httpReply(QTcpSocket* s, const QString& body);
  void serveAddinFile(QTcpSocket* s, const QString& path);
  void handleLine(const QString& raw);
  void touch();
  void ensureAddinRegistered();
  QString addinDir() const;
  void log(const QString& msg);

  QTcpServer* server_ = nullptr;
  QTimer*     ping_   = nullptr;
  QQueue<QPair<qint64, QString>> queue_;
  QElapsedTimer clock_;
  QElapsedTimer missingPosition_;
  QMap<QString, quint64> sequences_;
  bool whiteboard_ = false;
  bool    connected_ = false;
  qint64  lastSeen_  = 0;
  int     realPos_   = -1;
  int     deck_      = -1;
};
