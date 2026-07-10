import SwiftUI
import PhotosUI

struct HomeScreen: View {
    @Environment(\.dogTagColors) var c
    @ObservedObject private var store = LocalStore.shared
    @ObservedObject private var photos = PetPhotoStore.shared
    let onScan: () -> Void
    @State private var expanded: CredentialGroup? = nil
    @State private var selectedPetId: String? = nil
    @State private var detailCred: Credential? = nil

    // Pet-photo editing (local, UI-only). All three act on `currentPet`.
    @State private var photoDialog = false
    @State private var showLibraryPicker = false
    @State private var pickedItem: PhotosPickerItem? = nil
    @State private var showCamera = false

    private var currentPet: Pet? {
        store.pets.first { $0.dogTagId == selectedPetId } ?? store.pets.first
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text("Dog Tags").font(.system(size: 26, weight: .bold)).foregroundColor(c.onBackground)
                    Spacer()
                    // Labeled scan affordance — icon + "Scan" reads unambiguously as "scan a QR"
                    // (the old bare "+" over the photo read as "add" and confused the action).
                    Button(action: onScan) {
                        HStack(spacing: 6) {
                            Image(systemName: "qrcode.viewfinder").font(.system(size: 15, weight: .semibold))
                            Text("Scan").font(.system(size: 14, weight: .semibold))
                        }
                        .foregroundColor(c.onAccent)
                        .padding(.horizontal, 14).padding(.vertical, 9)
                        .background(Capsule().fill(c.accent))
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Scan a QR code")
                }

                if store.pets.isEmpty {
                    EmptyStateCard(
                        title: "No pets yet",
                        message: "Scan a vet or groomer's QR to import your dog's first verified record — your pets appear here automatically.",
                        onScan: onScan)
                } else {
                    if store.pets.count > 1 {
                        PetChips(pets: store.pets, selectedId: currentPet?.dogTagId) { selectedPetId = $0 }
                    }
                    if let pet = currentPet {
                        petPhotoCard(pet)
                        petIdentity(pet)
                        let petCreds = store.credentials.filter { $0.dogTagId == pet.dogTagId }
                        SectionTitle(text: "Credentials", trailing: "\(petCreds.count) total")
                        if petCreds.isEmpty {
                            EmptyStateCard(title: "No credentials yet",
                                           message: "Scan a vet's QR to import a record for \(pet.name).",
                                           onScan: onScan)
                        } else {
                            groupCard(.health, icon: "heart.fill", tint: c.healthTint, iconTint: c.danger, creds: petCreds)
                            groupCard(.service, icon: "shield.fill", tint: c.serviceTint, iconTint: c.success, creds: petCreds)
                            groupCard(.travel, icon: "airplane", tint: c.travelTint, iconTint: Color(hex: 0x2F6BFF), creds: petCreds)
                        }
                    }
                }
                Spacer(minLength: 24)
            }
            .padding(20)
        }
        .sheet(item: $detailCred) { cred in
            CredentialDetailScreen(cred: cred).environment(\.dogTagColors, c)
        }
        // Local pet-photo editing (acts on the selected pet). Library picker + camera + remove.
        .confirmationDialog("Pet photo", isPresented: $photoDialog, titleVisibility: .visible) {
            Button("Choose from Library") { showLibraryPicker = true }
            if UIImagePickerController.isSourceTypeAvailable(.camera) {
                Button("Take Photo") { showCamera = true }
            }
            if let pet = currentPet, photos.hasImage(for: pet.dogTagId) {
                Button("Remove Photo", role: .destructive) { photos.removeImage(for: pet.dogTagId) }
            }
            Button("Cancel", role: .cancel) {}
        }
        .photosPicker(isPresented: $showLibraryPicker, selection: $pickedItem, matching: .images)
        .onChange(of: pickedItem) { item in loadPickedPhoto(item) }
        .sheet(isPresented: $showCamera) {
            CameraPicker { image in
                if let id = currentPet?.dogTagId { photos.setImage(image, for: id) }
            }
            .ignoresSafeArea()
        }
    }

    /// Decode the PhotosPicker selection into a UIImage and store it against the active pet.
    private func loadPickedPhoto(_ item: PhotosPickerItem?) {
        guard let item, let id = currentPet?.dogTagId else { return }
        Task {
            if let data = try? await item.loadTransferable(type: Data.self),
               let image = UIImage(data: data) {
                await MainActor.run { photos.setImage(image, for: id) }
            }
            await MainActor.run { pickedItem = nil }
        }
    }

    private func petIdentity(_ pet: Pet) -> some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 2) {
                Text("NAME").font(.system(size: 11, weight: .semibold)).foregroundColor(c.muted)
                Text(pet.name).font(.system(size: 22, weight: .bold)).foregroundColor(c.onBackground)
                InlineCopyText(text: "DogTag #\(pet.dogTagId)", copyValue: pet.dogTagId)
            }
            Spacer()
            if !pet.breed.isEmpty {
                VStack(alignment: .trailing, spacing: 2) {
                    Text("BREED").font(.system(size: 11, weight: .semibold)).foregroundColor(c.muted)
                    Text(pet.breed).font(.system(size: 15, weight: .semibold)).foregroundColor(c.onBackground)
                    if !pet.ageLabel.isEmpty { Text(pet.ageLabel).font(.system(size: 12)).foregroundColor(c.muted) }
                }
            }
        }
    }

    /// The prominent pet portrait for the active dog-tag. Shows the locally-stored photo when set, a
    /// tappable silhouette placeholder otherwise, with a camera button to set/change it. The photo is
    /// device-local only (see `PetPhotoStore`).
    private func petPhotoCard(_ pet: Pet) -> some View {
        let image = photos.image(for: pet.dogTagId)
        return ZStack {
            RoundedRectangle(cornerRadius: 24).fill(c.accent.opacity(0.12))
                .aspectRatio(1.3, contentMode: .fit)
                .frame(maxWidth: .infinity)
            photoCircle(image)
                .frame(width: 200, height: 200)
                .overlay(alignment: .bottomTrailing) {
                    Button { photoDialog = true } label: {
                        ZStack {
                            Circle().fill(c.accent).frame(width: 46, height: 46)
                                .overlay(Circle().stroke(c.surface, lineWidth: 3))
                            Image(systemName: "camera.fill").foregroundColor(c.onAccent).font(.system(size: 17))
                        }
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(image == nil ? "Add pet photo" : "Change pet photo")
                    .offset(x: 4, y: 4)
                }
        }
        .frame(maxWidth: .infinity)
    }

    @ViewBuilder
    private func photoCircle(_ image: UIImage?) -> some View {
        if let image {
            Image(uiImage: image).resizable().scaledToFill()
                .frame(width: 200, height: 200)
                .clipShape(Circle())
                .overlay(Circle().stroke(c.surface, lineWidth: 4))
        } else {
            Button { photoDialog = true } label: {
                Circle().fill(c.surfaceVariant)
                    .overlay(
                        VStack(spacing: 8) {
                            Image(systemName: "pawprint.fill").font(.system(size: 52)).foregroundColor(c.accent.opacity(0.7))
                            Text("Add photo").font(.system(size: 13, weight: .semibold)).foregroundColor(c.muted)
                        }
                    )
            }
            .buttonStyle(.plain)
        }
    }

    private func groupCard(_ group: CredentialGroup, icon: String, tint: Color, iconTint: Color, creds: [Credential]) -> some View {
        let items = creds.filter { $0.group == group }
        return VStack(alignment: .leading, spacing: 10) {
            Button { expanded = (expanded == group) ? nil : group } label: {
                HStack {
                    ZStack {
                        Circle().fill(c.surface).frame(width: 38, height: 38)
                        Image(systemName: icon).foregroundColor(iconTint).font(.system(size: 18))
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text(group.title).font(.system(size: 15, weight: .semibold)).foregroundColor(c.onBackground)
                        Text("\(items.count) record\(items.count == 1 ? "" : "s")").font(.system(size: 12)).foregroundColor(c.muted)
                    }
                    Spacer()
                    Image(systemName: "chevron.right").foregroundColor(c.muted)
                }
            }
            .buttonStyle(.plain)
            if expanded == group {
                ForEach(items) { cred in
                    Button { detailCred = cred } label: {
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(cred.title).font(.system(size: 14, weight: .semibold)).foregroundColor(c.onBackground)
                                Text("\(cred.recordType) · \(cred.verdict)").font(.system(size: 12)).foregroundColor(c.muted)
                                if !cred.issuer.isEmpty { Text(cred.issuer).font(.system(size: 11)).foregroundColor(c.muted) }
                            }
                            Spacer()
                            Image(systemName: "chevron.right").foregroundColor(c.muted).font(.system(size: 12))
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(12)
                        .background(RoundedRectangle(cornerRadius: 12).fill(c.surface))
                    }.buttonStyle(.plain)
                }
            }
        }
        .padding(16)
        .background(RoundedRectangle(cornerRadius: 16).fill(tint))
    }
}

/// A horizontal chip row used to switch the active pet (Home). Each chip carries the pet's local photo
/// (or a paw placeholder) so switching the active dog-tag is visual, not just textual.
struct PetChips: View {
    @Environment(\.dogTagColors) var c
    @ObservedObject private var photos = PetPhotoStore.shared
    let pets: [Pet]
    let selectedId: String?
    let onSelect: (String?) -> Void
    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(pets) { p in
                    let sel = p.dogTagId == selectedId
                    Button { onSelect(p.dogTagId) } label: {
                        HStack(spacing: 8) {
                            chipAvatar(for: p, selected: sel)
                            Text(p.name).font(.system(size: 13, weight: .semibold))
                                .foregroundColor(sel ? c.onAccent : c.onBackground)
                        }
                        .padding(.leading, 6).padding(.trailing, 14).padding(.vertical, 6)
                        .background(Capsule().fill(sel ? c.accent : c.surfaceVariant))
                    }.buttonStyle(.plain)
                }
            }
        }
    }

    @ViewBuilder
    private func chipAvatar(for pet: Pet, selected: Bool) -> some View {
        if let image = photos.image(for: pet.dogTagId) {
            Image(uiImage: image).resizable().scaledToFill()
                .frame(width: 26, height: 26).clipShape(Circle())
        } else {
            Circle().fill((selected ? c.onAccent : c.accent).opacity(0.22))
                .frame(width: 26, height: 26)
                .overlay(Image(systemName: "pawprint.fill").font(.system(size: 12))
                    .foregroundColor(selected ? c.onAccent : c.accent))
        }
    }
}

/// A `UIImagePickerController` bridge for taking a pet photo with the camera. Only offered when a
/// camera is present (never on the Simulator). Library selection uses SwiftUI's `PhotosPicker`.
struct CameraPicker: UIViewControllerRepresentable {
    let onImage: (UIImage) -> Void
    @Environment(\.dismiss) private var dismiss

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.delegate = context.coordinator
        return picker
    }

    func updateUIViewController(_ vc: UIImagePickerController, context: Context) {}

    final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
        let parent: CameraPicker
        init(_ parent: CameraPicker) { self.parent = parent }

        func imagePickerController(_ picker: UIImagePickerController,
                                   didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
            if let image = info[.originalImage] as? UIImage { parent.onImage(image) }
            parent.dismiss()
        }
        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) { parent.dismiss() }
    }
}

/// A reusable empty-state card (mirrors Android EmptyState).
struct EmptyStateCard: View {
    @Environment(\.dogTagColors) var c
    let title: String
    let message: String
    let onScan: () -> Void
    var body: some View {
        VStack(spacing: 10) {
            ZStack {
                Circle().fill(c.surfaceVariant).frame(width: 56, height: 56)
                Image(systemName: "qrcode.viewfinder").foregroundColor(c.accent).font(.system(size: 26))
            }
            Text(title).font(.system(size: 16, weight: .bold)).foregroundColor(c.onBackground)
            Text(message).font(.system(size: 13)).foregroundColor(c.muted).multilineTextAlignment(.center)
            Button(action: onScan) {
                Text("Scan a QR").font(.system(size: 13, weight: .semibold)).foregroundColor(c.onAccent)
                    .padding(.horizontal, 18).padding(.vertical, 10)
                    .background(Capsule().fill(c.accent))
            }.buttonStyle(.plain)
        }
        .frame(maxWidth: .infinity)
        .padding(20)
        .background(RoundedRectangle(cornerRadius: 16).fill(c.surface))
    }
}
