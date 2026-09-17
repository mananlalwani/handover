plugins {
    id("com.android.application")
}

android {
    namespace = "org.handover.android"
    compileSdk = 37

    defaultConfig {
        applicationId = "org.handover.android"
        minSdk = 26
        targetSdk = 35
        versionCode = 3
        versionName = "0.2.1"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation("org.jetbrains.kotlin:kotlin-stdlib")
    testImplementation("junit:junit:4.13.2")
}
