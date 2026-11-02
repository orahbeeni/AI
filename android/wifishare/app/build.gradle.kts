plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.omar.wifishare"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.omar.wifishare"
        minSdk = 29
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // Personal sideloaded app: signed with the local debug key. Updates must be built with the same key.
            signingConfig = signingConfigs.getByName("debug")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    buildFeatures {
        compose = true
    }
}

// The engine is the wifishare Rust crate, built for the phone with cargo-ndk into jniLibs.
val cargoBuild by tasks.registering(Exec::class) {
    description = "Builds the Rust engine (rust/) for arm64 phones and x86_64 emulators"
    workingDir = rootProject.file("rust")
    commandLine(
        "cargo", "ndk", "-t", "arm64-v8a", "-t", "x86_64", "--platform", "29",
        "-o", project.file("src/main/jniLibs").path, "build", "--release",
    )
    inputs.dir(rootProject.file("rust/src"))
    inputs.dir(rootProject.file("../../wifishare/src"))
    inputs.file(rootProject.file("rust/Cargo.toml"))
    outputs.dir(project.file("src/main/jniLibs"))
}
tasks.named("preBuild") { dependsOn(cargoBuild) }

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.10.01")
    implementation(composeBom)
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
}
