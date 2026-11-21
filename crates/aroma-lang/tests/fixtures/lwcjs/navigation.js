import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
go(recordId) { this[NavigationMixin.Navigate]({ type: "standard__recordPage", attributes: { recordId } }); }
}
