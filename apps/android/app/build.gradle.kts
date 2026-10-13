import java.util.Properties

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

// Version policy (ADR-058): versionName is MAJOR.MINOR.PATCH and versionCode is derived from
// it, so each release has a unique, increasing code that does not depend on the build
// machine, the clock or git history (F-Droid builds from source and must get the same code).
val squibVersion = "0.1.0"
val squibVersionCode = squibVersion.split(".").map(String::toInt).let { (major, minor, patch) ->
    require(minor < 100 && patch < 100) { "minor and patch must be below 100" }
    major * 10_000 + minor * 100 + patch
}

// Release signing comes only from outside the repository: a git-ignored
// apps/android/keystore.properties (storeFile, storePassword, keyAlias, keyPassword) or the
// SQUIB_KEYSTORE, SQUIB_KEYSTORE_PASSWORD, SQUIB_KEY_ALIAS and SQUIB_KEY_PASSWORD environment
// variables. With neither, the release APK is built unsigned (as in CI).
val keystoreProps = Properties().apply {
    rootProject.file("keystore.properties").takeIf { it.isFile }?.inputStream()?.use { load(it) }
}
fun signingValue(key: String, env: String): String? = keystoreProps.getProperty(key) ?: System.getenv(env)
val releaseStoreFile = signingValue("storeFile", "SQUIB_KEYSTORE")
// Local testing only: sign the minified release with the debug key so it can be installed on
// an emulator. Never used for a build meant for anyone else.
val signReleaseWithDebugKey = (findProperty("squibSignReleaseWithDebugKey") as String?) == "true"

android {
    // Provisional identifiers: the reverse-domain application ID is an open owner
    // decision (docs/squib/README). See docs/squib/decisions/ADR-014.
    namespace = "net.digitalgrease.squib"
    compileSdk = 37
    // Lets AGP strip debug symbols from the Rust library in the APK (unstripped copies stay
    // under build/intermediates for symbolication).
    ndkVersion = "28.2.13676358"
    // Instrumented tests normally run against debug; -PsquibTestBuildType=release runs the
    // same suite against the minified build to check the keep rules (ADR-058).
    testBuildType = (findProperty("squibTestBuildType") as String?) ?: "debug"

    defaultConfig {
        applicationId = "net.digitalgrease.squib"
        minSdk = 26
        targetSdk = 36
        versionCode = squibVersionCode
        versionName = squibVersion
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // Long opt-in tests (the 30-minute capture soak) stay out of the default suite; see
        // CaptureSoakTest for how to run them.
        testInstrumentationRunnerArguments["notAnnotation"] = "net.digitalgrease.squib.Soak"
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    sourceSets {
        getByName("main").kotlin.directories += "src/main/generated/uniffi"
    }

    signingConfigs {
        if (releaseStoreFile != null) {
            create("release") {
                storeFile = file(releaseStoreFile)
                storePassword = signingValue("storePassword", "SQUIB_KEYSTORE_PASSWORD")
                keyAlias = signingValue("keyAlias", "SQUIB_KEY_ALIAS")
                keyPassword = signingValue("keyPassword", "SQUIB_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            // Building is not publishing: distribution needs separate authorization.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            testProguardFiles("test-proguard-rules.pro")
            if (testBuildType == "release") proguardFiles("proguard-test-target.pro")
            signingConfig = when {
                signReleaseWithDebugKey -> signingConfigs.getByName("debug")
                releaseStoreFile != null -> signingConfigs.getByName("release")
                else -> null
            }
        }
    }

    // F-Droid refuses APKs carrying Google's encrypted dependency-metadata block.
    dependenciesInfo {
        includeInApk = false
        includeInBundle = false
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
    androidTestImplementation(libs.compose.ui.test.junit4.accessibility)
}
