import QtQuick
import QtQuick.Layouts

ColumnLayout {
    property string title: ""
    property string description: ""

    spacing: 8

    Text {
        Layout.fillWidth: true
        text: parent.title
        textFormat: Text.PlainText
        color: "#edf1f7"
        font.pixelSize: 16
        font.weight: Font.DemiBold
        wrapMode: Text.WordWrap
    }

    Text {
        Layout.fillWidth: true
        text: parent.description
        visible: text.length > 0
        textFormat: Text.PlainText
        color: "#aab7c8"
        font.pixelSize: 13
        wrapMode: Text.WordWrap
        lineHeight: 1.25
    }

}
