import QtQuick
import QtQuick.Controls

ComboBox {
    id: control

    implicitHeight: 38
    implicitWidth: 220
    leftPadding: 12
    rightPadding: 30
    font.pixelSize: 13

    contentItem: Text {
        text: control.displayText
        font: control.font
        textFormat: Text.PlainText
        color: control.enabled ? "#edf1f7" : "#8291a6"
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }

    indicator: Text {
        x: control.width - width - 12
        anchors.verticalCenter: parent.verticalCenter
        text: "▾"
        color: "#aab7c8"
        font.pixelSize: 13
    }

    background: Rectangle {
        radius: 8
        color: "#202b3a"
        border.color: control.activeFocus ? "#9bc5f4" : "#354154"
    }

}
