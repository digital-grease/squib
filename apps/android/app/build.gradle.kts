plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.compose.compiler)
}

// Rust core: cross-compiled with cargo-ndk and bound with UniFFI before every build.
// Set -PsquibSkipRust=true to reuse previously built libraries (e.g. offline iteration).
val repoRoot = rootProject.projectDir.parentFile.parentFile
val rustProfile = (findProperty("squibRustProfile") as String?) ?: "release"
val buildRustCore = tasks.register<Exec>("buildRustCore") {
    group = "build"
    description = "Build libsquib_mobile.so for Android ABIs and generate Kotlin bindings"
    onlyIf { (findProperty("squibSkipRust") as String?) != "true" }
    workingDir = repoRoot
    commandLine("bash", "scripts/build-android-core.sh", rustProfile)
    inputs.dir(File(repoRoot, "crates"))
    inputs.file(File(repoRoot, "Cargo.lock"))
    inputs.file(File(repoRoot, "scripts/build-android-core.sh"))
    outputs.dir("src/main/jniLibs")
    outputs.dir("src/main/generated/uniffi")
}
tasks.named("preBuild") { dependsOn(buildRustCore) }

android {
    // Provisional identifiers: the reverse-domain application ID is an open owner
    // decision (docs/squib/README). See docs/squib/decisions/ADR-014.
    namespace = "net.digitalgrease.squib"
    compileSdk = 37

    defaultConfig {
        applicationId = "net.digitalgrease.squib"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0-m1"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    sourceSets {
        getByName("main").kotlin.directories += "src/main/generated/uniffi"
    }

    buildTypes {
        release {
            // No release signing or distribution is configured: publishing builds needs
            // separate authorization.
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
}

java {
    toolchain {
        languageVersion = JavaLanguageVersion.of(21)
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_21)
    }
}

dependencies {
    implementation(libs.core.ktx)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.lifecycle.viewmodel.compose)
    implementation(libs.activity.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.graphics)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.coroutines.android)
    implementation("${libs.jna.get()}@aar")
    debugImplementation(libs.compose.ui.tooling)
    debugImplementation(libs.compose.ui.test.manifest)

    testImplementation(libs.junit)
    androidTestImplementation(libs.junit.ext)
    androidTestImplementation(libs.test.runner)
    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.ui.test.junit4)
}
